//! A "packed archetype" storage model.
//!
//! Any combination of types of components can be attached to each entity
//! in a [world](../world/struct.World.html). Storing the (potentially
//! unique) set of component values for each entity in a manor which is
//! efficient to search and access is the responsibility of the ECS libary.
//!
//! Legion achieves this via the use of "archetypes". Archetypes are a
//! collection of entities who all have exactly the same set of component
//! types attached. By storing these together, we can perform filtering
//! operations at the archetype level without needing to ever inspect each
//! individual entity. Archetypes also allow us to store contiguous and
//! ordered arrays of each component. For example, all `Position` components
//! for all entities in an archetype are stored together in one array, and
//! can be returned from queries as a slice. All component arrays for an
//! archetype are stored in the same order and are necessarily the same
//! length, allowing us to index them together to access the components for
//! a single entity.
//!
//! Because these components are stored contiguously in memory, iterating
//! through the components in an archetype is extremely performant as
//! it offers perfect cache coherence. By storing each component type in
//! its own array, we only need to access the memory containing components
//! actually reqested by the query's view (see the
//! [query module](../query/index.html)).
//!
//! One of the disadvantages of archetypes is that there are discontinuities
//! between component arrays of different archetypes. In practise this causes
//! approximately one additional L2/3 cache miss per unique entity layout that
//! exists among the result set of a query.
//!
//! Legion mitigates this by conservatively packing archetype component
//! slices next to each other. A component slice is considered eligable
//! for packing if its components have remained stable for some time (i.e no
//! entities have been added or removed from the archetype recently) and
//! and estimate of potential saved cache misses passes a "worthwhile"
//! threshold.
//!
//! By default, legion will pack component slices in the order in which
//! the archetypes were created. This matches the order in which queries will
//! attempt to access each slice. However, most queries do not access all
//! archetypes that contain a certain component - more likely they will skip
//! past many archetypes due to other filters (such as only being interested
//! in archetypes which also contain another specific component).
//!
//! We can provide hints to a world about how it should pack archetypes by
//! declaring groups with the world's [options](../world/struct.WorldOptions.html)
//! when creating the world. Component groups can be used to accelerate the
//! largest and most common queries by optmizing data layout for those queries.
//!
//! Each component type in a world may belong to precisely one group. A group is
//! a set of components which are frequently queried for together. Queries which
//! match a group will not suffer from performance loss due to archetype
//! fragmentation.
//!
//! Each group implicitly also defines sub-groups, such that the group
//! `(A, B, C, D)` also defines the groups `(A, B, C)` and `(A, B)`.
//!
//! Groups are defined before constructing a world and are passed in the world's
//! options.
//!
//! ```
//! struct A;
//! struct B;
//! struct C;
//!
//! let group = <(A, B, C)>::to_group();
//! let options = WorldOptions { groups: vec![group] };
//! let world = World::with_options(options);
//! ```

use crate::hash::ComponentTypeIdHasher;
use downcast_rs::{impl_downcast, Downcast};
use std::{
    collections::{HashMap, HashSet},
    hash::BuildHasherDefault,
    ops::{Deref, DerefMut, Index, IndexMut},
    sync::atomic::{AtomicU64, Ordering},
};

mod archetype;
mod component;
mod group;
mod index;
mod packed;
mod slicevec;

pub use archetype::{Archetype, ArchetypeIndex, EntityLayout};
pub use component::{Component, ComponentTypeId};
pub use group::{Group, GroupDef, GroupSource, SubGroup};
pub use index::SearchIndex;

/// Contains information about the type of a component.
pub struct ComponentMeta {
    size: usize,
    align: usize,
    drop_fn: Option<fn(*mut u8)>,
}

impl ComponentMeta {
    /// Returns the component meta of component type `T`.
    pub fn of<T: Component>() -> Self {
        ComponentMeta {
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(|ptr| unsafe { std::ptr::drop_in_place(ptr as *mut T) })
            } else {
                None
            },
        }
    }

    /// Returns the size of the component.
    pub fn size(&self) -> usize { self.size }

    /// Returns the alignment of the component.
    pub fn align(&self) -> usize { self.align }

    /// Drops the component.
    ///
    /// # Safety
    /// The caller must ensure that the memory location refered to by `value` is
    /// not accessed again before it is re-initialized.
    pub unsafe fn drop(&self, value: *mut u8) {
        if let Some(drop_fn) = self.drop_fn {
            drop_fn(value)
        }
    }
}

/// The index of a component within an archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ComponentIndex(pub(crate) usize);

/// A world epoch. Epochs are incremented each time a world is packed, and are used
/// by the packing heuristics as a measure of age.
pub type Epoch = u64;

/// The version of a component slice. Versions are incremented when the sliace is
/// accessed mutably.
pub type Version = u64;

static COMPONENT_VERSION: AtomicU64 = AtomicU64::new(0);
pub(crate) fn next_component_version() -> u64 { COMPONENT_VERSION.fetch_add(1, Ordering::SeqCst) }

/// A storage location for component data slices. Each component storage may hold once slice for
/// each archetype inserted into the storage. The type of component stored is not known statically.
pub trait UnknownComponentStorage: Downcast + Send + Sync {
    /// Notifies the storage of the start of a new epoch.
    fn increment_epoch(&mut self);

    /// Inserts a new empty component slice for an archetype into this storage.
    fn insert_archetype(&mut self, archetype: ArchetypeIndex, index: Option<usize>);

    /// Moves an archetype's component slice to a new storage.
    fn transfer_archetype(
        &mut self,
        src_archetype: ArchetypeIndex,
        dst_archetype: ArchetypeIndex,
        dst: &mut dyn UnknownComponentStorage,
    );

    /// Moves a component to a new storage.
    fn transfer_component(
        &mut self,
        src_archetype: ArchetypeIndex,
        src_component: ComponentIndex,
        dst_archetype: ArchetypeIndex,
        dst: &mut dyn UnknownComponentStorage,
    );

    /// Moves a component from one archetype to another.
    fn move_component(
        &mut self,
        source: ArchetypeIndex,
        index: ComponentIndex,
        dst: ArchetypeIndex,
    );

    /// Removes a component from an archetype slice, swapping it with the last component in the slice.
    fn swap_remove(&mut self, archetype: ArchetypeIndex, index: ComponentIndex);

    /// Packs archetype slices.
    fn pack(&mut self, epoch_threshold: Epoch) -> usize;

    /// A heuristic estimating cache misses for an iteration through all components due to archetype fragmentation.
    fn fragmentation(&self) -> f32;

    /// Returns the component metadata.
    fn element_vtable(&self) -> ComponentMeta;

    /// Returns a pointer to the given archetype's component slice.
    fn get_raw(&self, archetype: ArchetypeIndex) -> Option<(*const u8, usize)>;

    /// Returns a pointer to the given archetype's component slice.
    ///
    /// # Safety
    /// The caller is responsible for ensuring that they have exclusive access to the given archetype's slice.
    unsafe fn get_mut_raw(&self, archetype: ArchetypeIndex) -> Option<(*mut u8, usize)>;

    /// Writes new components into the given archetype's component slice via a memcopy.
    ///
    /// # Safety
    /// `ptr` must point to a valid array of the correct component type of length at least as long as `len`.
    /// The data in this array will be memcopied into the world's internal storage.
    /// If the component type is not `Copy`, then the caller must ensure that the memory
    /// copied is not accessed until it is re-initialized. It is recommended to immediately
    /// `std::mem::forget` the source after calling `extend_memcopy_raw`.
    unsafe fn extend_memcopy_raw(&mut self, archetype: ArchetypeIndex, ptr: *const u8, len: usize);
}
impl_downcast!(UnknownComponentStorage);

/// An accessor for a shared slice reference of components for a single archetype.
pub struct ComponentSlice<'a, T: Component> {
    pub(crate) components: &'a [T],
    pub(crate) version: &'a Version,
}

impl<'a, T: Component> ComponentSlice<'a, T> {
    pub(crate) fn new(components: &'a [T], version: &'a Version) -> Self {
        Self {
            components,
            version,
        }
    }

    /// Converts this slice into its inner value.
    pub fn into_slice(self) -> &'a [T] { self.components }
}

impl<'a, T: Component> Into<&'a [T]> for ComponentSlice<'a, T> {
    fn into(self) -> &'a [T] { self.components }
}

impl<'a, T: Component> Deref for ComponentSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target { &self.components }
}

impl<'a, T: Component> Index<ComponentIndex> for ComponentSlice<'a, T> {
    type Output = T;
    fn index(&self, index: ComponentIndex) -> &Self::Output { &self.components[index.0] }
}

/// An accessor for a mutable slice reference of components for a single archetype.
pub struct ComponentSliceMut<'a, T: Component> {
    // todo would be better if these were private and we controlled version increments more centrally
    pub(crate) components: &'a mut [T],
    pub(crate) version: &'a mut Version,
}

impl<'a, T: Component> ComponentSliceMut<'a, T> {
    pub(crate) fn new(components: &'a mut [T], version: &'a mut Version) -> Self {
        Self {
            components,
            version,
        }
    }

    /// Converts this slice into its inner value.
    /// This increments the slice's version.
    pub fn into_slice(self) -> &'a mut [T] {
        *self.version = next_component_version();
        self.components
    }
}

impl<'a, T: Component> Deref for ComponentSliceMut<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target { &self.components }
}

impl<'a, T: Component> Index<ComponentIndex> for ComponentSliceMut<'a, T> {
    type Output = T;
    fn index(&self, index: ComponentIndex) -> &Self::Output { &self.components[index.0] }
}

impl<'a, T: Component> IndexMut<ComponentIndex> for ComponentSliceMut<'a, T> {
    fn index_mut(&mut self, index: ComponentIndex) -> &mut Self::Output {
        &mut self.components[index.0]
    }
}

/// A storage location for component data slices. Each component storage may hold once slice for
/// each archetype inserted into the storage.
pub trait ComponentStorage<'a, T: Component>: UnknownComponentStorage + Default {
    /// An iterator of shared archetype slice references.
    type Iter: Iterator<Item = ComponentSlice<'a, T>>;

    /// An iterator of mutable archetype slice references.
    type IterMut: Iterator<Item = ComponentSliceMut<'a, T>>;

    /// Returns the number of archetype slices stored.
    fn len(&self) -> usize;

    /// Copies new components into the specified archetype slice.
    ///
    /// # Safety
    /// The components located at `ptr` are memcopied into the storage. If `T` is not `Copy`, then the
    /// previous memory location should no longer be accessed.
    unsafe fn extend_memcopy(&mut self, archetype: ArchetypeIndex, ptr: *const T, len: usize);

    /// Ensures that the given spare capacity is available for component insertions. This is a performance hint and
    /// should not be required before `extend_memcopy` is called.
    fn ensure_capacity(&mut self, archetype: ArchetypeIndex, space: usize);

    /// Gets the component slice for the specified archetype.
    fn get(&'a self, archetype: ArchetypeIndex) -> Option<ComponentSlice<'a, T>>;

    /// Gets a mutable component slice for the specified archetype.
    ///
    /// # Safety
    /// Ensure that the requested archetype slice is not concurrently borrowed anywhere else such that memory
    /// is not mutably aliased.
    unsafe fn get_mut(&'a self, archetype: ArchetypeIndex) -> Option<ComponentSliceMut<'a, T>>;

    /// Iterates through all archetype component slices.
    fn iter(&'a self, start_inclusive: usize, end_exclusive: usize) -> Self::Iter;

    /// Iterates through all mutable archetype component slices.
    ///
    /// # Safety
    /// Ensure that all requested archetype slices are not concurrently borrowed anywhere else such that memory
    /// is not mutably aliased.
    unsafe fn iter_mut(&'a self, start_inclusive: usize, end_exclusive: usize) -> Self::IterMut;
}

/// Contains the storages for all component types in a world.
#[derive(Default)]
pub struct Components {
    storages: HashMap<
        ComponentTypeId,
        Box<dyn UnknownComponentStorage>,
        BuildHasherDefault<ComponentTypeIdHasher>,
    >,
}

impl Components {
    /// Gets or inserts the storage for the given component type.
    pub fn get_or_insert_with<F>(
        &mut self,
        type_id: ComponentTypeId,
        mut create: F,
    ) -> &mut dyn UnknownComponentStorage
    where
        F: FnMut() -> Box<dyn UnknownComponentStorage>,
    {
        let cell = self.storages.entry(type_id).or_insert_with(|| create());
        cell.deref_mut()
    }

    /// Returns the storage for the given component type.
    pub fn get(&self, type_id: ComponentTypeId) -> Option<&dyn UnknownComponentStorage> {
        self.storages.get(&type_id).map(|cell| cell.deref())
    }

    /// Returns the storage for the given component type.
    pub fn get_downcast<T: Component>(&self) -> Option<&T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        self.get(type_id).and_then(|storage| storage.downcast_ref())
    }

    /// Returns the storage for the given component type.
    pub fn get_mut(
        &mut self,
        type_id: ComponentTypeId,
    ) -> Option<&mut dyn UnknownComponentStorage> {
        self.storages.get_mut(&type_id).map(|cell| cell.deref_mut())
    }

    /// Returns the storage for the given component type.
    pub fn get_downcast_mut<T: Component>(&mut self) -> Option<&mut T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        self.get_mut(type_id)
            .and_then(|storage| storage.downcast_mut())
    }

    /// Returns a writer for writing to multiple component storages.
    pub fn get_multi_mut(&mut self) -> MultiMut { MultiMut::new(self) }

    /// Repacks all component storages.
    pub fn pack(&mut self, options: &PackOptions) {
        let mut total_moved_bytes = 0;
        for storage in self.iter_storages_mut() {
            if storage.fragmentation() >= options.fragmentation_threshold {
                total_moved_bytes += storage.pack(options.stability_threshold);
            }

            if total_moved_bytes >= options.maximum_iteration_size {
                break;
            }
        }

        for storage in self.iter_storages_mut() {
            storage.increment_epoch();
        }
    }

    fn iter_storages_mut(&mut self) -> impl Iterator<Item = &mut dyn UnknownComponentStorage> {
        self.storages.iter_mut().map(|(_, cell)| cell.deref_mut())
    }
}

impl std::fmt::Debug for Components {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.storages.keys()).finish()
    }
}

/// Describes how to perform a component pack operation.
#[derive(Copy, Clone, Debug)]
pub struct PackOptions {
    /// The number of frames that an archetype has to remain stable before it
    /// will be considered a candidate for packing.
    pub stability_threshold: u64,

    /// The estimated number of cache misses due to fragmentation per entity
    /// that would be saved by a repack before a component storage may consider
    /// repacking itself.
    pub fragmentation_threshold: f32,

    /// The target maximum number of entities to move during a repack before
    /// the pack is halted.
    pub maximum_iteration_size: usize,
}

impl PackOptions {
    /// Force a repack.
    pub fn force() -> Self {
        Self {
            stability_threshold: 0,
            fragmentation_threshold: 0.0,
            maximum_iteration_size: usize::MAX,
        }
    }
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            stability_threshold: 120,
            fragmentation_threshold: 1.0 / 64.0,
            maximum_iteration_size: 4 * 1024 * 1024,
        }
    }
}

/// Provides mutable access to multiple different component storages from a single world.
pub struct MultiMut<'a> {
    components: &'a mut Components,
    #[cfg(debug_assertions)]
    claimed: HashSet<ComponentTypeId, BuildHasherDefault<ComponentTypeIdHasher>>,
}

impl<'a> MultiMut<'a> {
    fn new(components: &'a mut Components) -> Self {
        Self {
            components,
            #[cfg(debug_assertions)]
            claimed: HashSet::default(),
        }
    }

    /// Claims exclusive access to a component storage.
    ///
    /// # Safety
    /// The caller must ensure that each component type is only claimed once, as doing otherwise
    /// may result in mutable aliases of the component storage. This is validated in debug builds.
    pub unsafe fn claim<T: Component>(&mut self) -> Option<&'a mut T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        #[cfg(debug_assertions)]
        {
            assert!(!self.claimed.contains(&type_id));
            self.claimed.insert(type_id);
        }
        // Self::extend_lifetime extends the local borrow up to 'a.
        // This is highly unsafe as it would allow aliasing a mutable borrow
        // by calling claim() multiple times for the same component.
        // However, the caller is responsible for not doing this as part of claim's safety rules.
        // We validate this in debug builds.
        self.components
            .storages
            .get_mut(&type_id)
            .and_then(|cell| Self::extend_lifetime(cell).downcast_mut())
    }

    /// Claims exclusive access to a component storage.
    ///
    /// # Safety
    /// The caller must ensure that each component type is only claimed once, as doing otherwise
    /// may result in mutable aliases of the component storage. This is validated in debug builds.
    pub unsafe fn claim_unknown(
        &mut self,
        type_id: ComponentTypeId,
    ) -> Option<&'a mut dyn UnknownComponentStorage> {
        #[cfg(debug_assertions)]
        {
            assert!(!self.claimed.contains(&type_id));
            self.claimed.insert(type_id);
        }
        // Self::extend_lifetime extends the local borrow up to 'a.
        // This is highly unsafe as it would allow aliasing a mutable borrow
        // by calling claim_unknown() multiple times for the same component.
        // However, the caller is responsible for not doing this as part of claim_unknown's safety rules.
        // We validate this in debug builds.
        self.components
            .storages
            .get_mut(&type_id)
            .map(|cell| Self::extend_lifetime(cell).deref_mut())
    }

    unsafe fn extend_lifetime<'b, T>(value: &'b mut T) -> &'a mut T {
        std::mem::transmute::<&'b mut T, &'a mut T>(value)
    }
}
