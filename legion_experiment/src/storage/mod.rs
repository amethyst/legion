use crate::hash::ComponentTypeIdHasher;
use archetype::ArchetypeIndex;
use component::{Component, ComponentTypeId};
use downcast_rs::{impl_downcast, Downcast};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet},
    hash::BuildHasherDefault,
    ops::{Deref, DerefMut, Index, IndexMut},
};

pub mod archetype;
pub mod component;
pub mod group;
pub mod index;
pub mod packed;
pub mod slicevec;

pub struct ComponentMeta {
    size: usize,
    align: usize,
    drop_fn: Option<fn(*mut u8)>,
}

impl ComponentMeta {
    /// Gets the component meta of component type `T`.
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

    pub fn size(&self) -> usize { self.size }

    pub fn align(&self) -> usize { self.align }

    pub unsafe fn drop(&self, value: *mut u8) {
        if let Some(drop_fn) = self.drop_fn {
            drop_fn(value)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ComponentIndex(pub(crate) usize);

pub trait UnknownComponentStorage: Downcast {
    fn insert_archetype(&mut self, archetype: ArchetypeIndex, index: Option<usize>);
    fn move_archetype(
        &mut self,
        src_archetype: ArchetypeIndex,
        dst_archetype: ArchetypeIndex,
        dst: &mut dyn UnknownComponentStorage,
        dst_frame_counter: u64,
    );
    fn move_component(
        &mut self,
        epoch: u64,
        source: ArchetypeIndex,
        index: usize,
        dst: ArchetypeIndex,
    );
    fn swap_remove(&mut self, epoch: u64, archetype: ArchetypeIndex, index: usize);
    fn pack(&mut self, epoch_threshold: u64) -> usize;
    fn fragmentation(&self) -> f32;
    fn element_vtable(&self) -> ComponentMeta;
    fn get_raw(&self, archetype: ArchetypeIndex) -> Option<(*const u8, usize)>;
    unsafe fn get_mut_raw(&self, epoch: u64, archetype: ArchetypeIndex)
        -> Option<(*mut u8, usize)>;
    unsafe fn extend_memcopy_raw(
        &mut self,
        epoch: u64,
        archetype: ArchetypeIndex,
        ptr: *const u8,
        len: usize,
    );
}
impl_downcast!(UnknownComponentStorage);

pub struct ComponentSlice<'a, T: Component> {
    pub components: &'a [T],
    pub version: &'a u64,
}

impl<'a, T: Component> ComponentSlice<'a, T> {
    pub fn new(components: &'a [T], version: &'a u64) -> Self {
        Self {
            components,
            version,
        }
    }

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

pub struct ComponentSliceMut<'a, T: Component> {
    pub components: &'a mut [T],
    pub version: &'a mut u64,
}

impl<'a, T: Component> ComponentSliceMut<'a, T> {
    pub fn new(components: &'a mut [T], version: &'a mut u64) -> Self {
        Self {
            components,
            version,
        }
    }

    pub fn into_slice(self) -> &'a mut [T] { self.components }
}

impl<'a, T: Component> Into<&'a mut [T]> for ComponentSliceMut<'a, T> {
    fn into(self) -> &'a mut [T] { self.components }
}

impl<'a, T: Component> Deref for ComponentSliceMut<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target { &self.components }
}

impl<'a, T: Component> DerefMut for ComponentSliceMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.components }
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

pub trait ComponentStorage<'a, T: Component>: UnknownComponentStorage + Default {
    type Iter: Iterator<Item = ComponentSlice<'a, T>>;
    type IterMut: Iterator<Item = ComponentSliceMut<'a, T>>;

    fn len(&self) -> usize;

    /// Copies new components into the specified archetype slice.
    ///
    /// # Safety
    /// The components located at `ptr` are memcopied into the storage. If `T` is not `Copy`, then the
    /// previous memory location should no longer be accessed.
    unsafe fn extend_memcopy(
        &mut self,
        epoch: u64,
        archetype: ArchetypeIndex,
        ptr: *const T,
        len: usize,
    );

    /// Ensures that the given spare capacity is available for component insertions. This is a performance hint and
    /// should not be required before `extend_memcopy` is called.
    fn ensure_capacity(&mut self, epoch: u64, archetype: ArchetypeIndex, space: usize);

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

#[derive(Default)]
pub struct Components {
    storages: HashMap<
        ComponentTypeId,
        UnsafeCell<Box<dyn UnknownComponentStorage>>,
        BuildHasherDefault<ComponentTypeIdHasher>,
    >,
}

impl Components {
    pub fn get_or_insert_with<F>(
        &mut self,
        type_id: ComponentTypeId,
        mut create: F,
    ) -> &mut dyn UnknownComponentStorage
    where
        F: FnMut() -> Box<dyn UnknownComponentStorage>,
    {
        let cell = self
            .storages
            .entry(type_id)
            .or_insert_with(|| UnsafeCell::new(create()));
        unsafe { &mut *cell.get() }.deref_mut()
    }

    pub fn get(&self, type_id: ComponentTypeId) -> Option<&dyn UnknownComponentStorage> {
        self.storages
            .get(&type_id)
            .map(|cell| unsafe { &*cell.get() }.deref())
    }

    pub fn get_downcast<T: Component>(&self) -> Option<&T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        self.get(type_id).and_then(|storage| storage.downcast_ref())
    }

    pub fn get_mut(
        &mut self,
        type_id: ComponentTypeId,
    ) -> Option<&mut dyn UnknownComponentStorage> {
        self.storages
            .get_mut(&type_id)
            .map(|cell| unsafe { &mut *cell.get() }.deref_mut())
    }

    pub fn get_downcast_mut<T: Component>(&mut self) -> Option<&mut T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        self.get_mut(type_id)
            .and_then(|storage| storage.downcast_mut())
    }

    pub fn get_multi_mut(&mut self) -> MultiMut { MultiMut::new(self) }

    pub fn pack(&mut self, options: &PackOptions, epoch: u64) {
        let mut total_moved_bytes = 0;
        for (_, cell) in self.storages.iter_mut() {
            let storage = unsafe { &mut *cell.get() };

            if storage.fragmentation() >= options.fragmentation_threshold {
                total_moved_bytes += storage.pack(epoch - options.stability_threshold);
            }

            if total_moved_bytes >= options.maximum_iteration_size {
                break;
            }
        }
    }
}

impl std::fmt::Debug for Components {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.storages.keys()).finish()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct PackOptions {
    stability_threshold: u64,
    fragmentation_threshold: f32,
    maximum_iteration_size: usize,
}

impl PackOptions {
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

    pub unsafe fn claim<T: Component>(&mut self) -> Option<&'a mut T::Storage> {
        let type_id = ComponentTypeId::of::<T>();
        #[cfg(debug_assertions)]
        {
            assert!(!self.claimed.contains(&type_id));
            self.claimed.insert(type_id);
        }
        self.components
            .storages
            .get(&type_id)
            .and_then(|cell| { &mut *cell.get() }.downcast_mut())
    }

    pub unsafe fn claim_unknown(
        &mut self,
        type_id: ComponentTypeId,
    ) -> Option<&'a mut dyn UnknownComponentStorage> {
        #[cfg(debug_assertions)]
        {
            assert!(!self.claimed.contains(&type_id));
            self.claimed.insert(type_id);
        }
        self.components
            .storages
            .get(&type_id)
            .map(|cell| &mut **cell.get())
    }
}
