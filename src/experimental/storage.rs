use crate::experimental::borrow::Exclusive;
use crate::experimental::borrow::Shared;
use crate::experimental::borrow::{AtomicRefCell, Ref, RefMap, RefMapMut, RefMut};
use crate::experimental::entity::Entity;
use derivative::Derivative;
use smallvec::Drain;
use smallvec::SmallVec;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::mem::size_of;
use std::num::Wrapping;
use std::ptr::NonNull;
use std::slice::Iter;
use std::slice::IterMut;

/// A type ID identifying a component type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ComponentTypeId(TypeId);

impl ComponentTypeId {
    /// Gets the component type ID that represents type `T`.
    pub fn of<T: Component>() -> Self { ComponentTypeId(TypeId::of::<T>()) }
}

/// A type ID identifying a tag type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TagTypeId(TypeId);

impl TagTypeId {
    /// Gets the tag type ID that represents type `T`.
    pub fn of<T: Tag>() -> Self { TagTypeId(TypeId::of::<T>()) }
}

/// A `Component` is per-entity data that can be attached to a single entity.
pub trait Component: Copy + Send + Sync + 'static {}

/// A `Tag` is shared data that can be attached to multiple entities at once.
pub trait Tag: Copy + Send + Sync + PartialEq + 'static {}

impl<T: Copy + Send + Sync + 'static> Component for T {}
impl<T: Copy + Send + Sync + PartialEq + 'static> Tag for T {}

/// Stores slices of `ComponentTypeId`, each of which identifies the type of components
/// contained within the archetype of the same index.
#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct ComponentTypes(SliceVec<ComponentTypeId>);

/// Stores slices of `TagTypeId`, each of which identifies the type of tags
/// contained within the archetype of the same index.
#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct TagTypes(SliceVec<TagTypeId>);

impl ComponentTypes {
    /// Gets an iterator over all type ID slices.
    pub fn iter(&self) -> SliceVecIter<ComponentTypeId> { self.0.iter() }

    /// Gets the number of slices stored within the set.
    pub fn len(&self) -> usize { self.0.len() }

    /// Determines if the set is empty.
    pub fn is_empty(&self) -> bool { self.len() < 1 }
}

impl TagTypes {
    /// Gets an iterator over all type ID slices.
    pub fn iter(&self) -> SliceVecIter<TagTypeId> { self.0.iter() }

    /// Gets the number of slices stored within the set.
    pub fn len(&self) -> usize { self.0.len() }

    /// Determines if the set is empty.
    pub fn is_empty(&self) -> bool { self.len() < 1 }
}

/// A vector of slices.
///
/// Each slice is stored inline so as to be efficiently iterated through linearly.
#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct SliceVec<T> {
    data: Vec<T>,
    counts: Vec<usize>,
}

impl<T> SliceVec<T> {
    /// Gets the length of the vector.
    pub fn len(&self) -> usize { self.counts.len() }

    /// Determines if the vector is empty.
    pub fn is_empty(&self) -> bool { self.len() < 1 }

    /// Pushes a new slice onto the end of the vector.
    pub fn push<I: IntoIterator<Item = T>>(&mut self, items: I) {
        let mut count = 0;
        for item in items.into_iter() {
            self.data.push(item);
            count += 1;
        }
        self.counts.push(count);
    }

    /// Gets an iterator over all slices in the vector.
    pub fn iter(&self) -> SliceVecIter<T> {
        SliceVecIter {
            data: &self.data,
            counts: &self.counts,
        }
    }
}

/// An iterator over slices in a `SliceVec`.
pub struct SliceVecIter<'a, T> {
    data: &'a [T],
    counts: &'a [usize],
}

impl<'a, T> Iterator for SliceVecIter<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((count, remaining_counts)) = self.counts.split_first() {
            let (data, remaining_data) = self.data.split_at(*count);
            self.counts = remaining_counts;
            self.data = remaining_data;
            Some(data)
        } else {
            None
        }
    }
}

/// Stores all entity data for a `World`.
#[derive(Default)]
pub struct Storage {
    component_types: ComponentTypes,
    tag_types: TagTypes,
    chunks: Vec<ArchetypeData>,
}

impl Storage {
    /// Creates a new archetype.
    ///
    /// Returns the index of the newly created archetype and an exclusive reference to the
    /// achetype's data.
    pub fn alloc_archetype(&mut self, desc: &ArchetypeDescription) -> (usize, &mut ArchetypeData) {
        self.component_types
            .0
            .push(desc.components.iter().map(|(type_id, _)| *type_id));
        self.tag_types
            .0
            .push(desc.tags.iter().map(|(type_id, _)| *type_id));
        self.chunks
            .push(ArchetypeData::new(ArchetypeId(self.chunks.len()), desc));

        let index = self.chunks.len() - 1;
        (index, unsafe { self.data_unchecked_mut(index) })
    }

    /// Gets a vector of slices of all component types for all archetypes.
    ///
    /// Each slice contains the component types for the archetype at the corresponding index.
    pub fn component_types(&self) -> &ComponentTypes { &self.component_types }

    /// Gets a vector of slices of all tag types for all archetypes.
    ///
    /// Each slice contains the tag types for the archetype at the corresponding index.
    pub fn tag_types(&self) -> &TagTypes { &self.tag_types }

    /// Gets the entity data for the specified archetype.
    pub fn data(&self, archetype: usize) -> Option<&ArchetypeData> { self.chunks.get(archetype) }

    /// Gets the entity data for the specified archetype.
    pub fn data_mut(&mut self, archetype: usize) -> Option<&mut ArchetypeData> {
        self.chunks.get_mut(archetype)
    }

    /// Gets the entity data for the specified archetype.
    ///
    /// # Safety
    ///
    /// This method performs no bounds checking. Calling it with an out-of-bounds index is *undefined behavior*.
    pub unsafe fn data_unchecked(&self, archetype: usize) -> &ArchetypeData {
        self.chunks.get_unchecked(archetype)
    }

    /// Gets the entity data for the specified archetype.
    ///
    /// # Safety
    ///
    /// This method performs no bounds checking. Calling it with an out-of-bounds index is *undefined behavior*.
    pub unsafe fn data_unchecked_mut(&mut self, archetype: usize) -> &mut ArchetypeData {
        self.chunks.get_unchecked_mut(archetype)
    }
}

/// Stores metadata decribing the type of a tag.
#[derive(Copy, Clone)]
pub struct TagMeta {
    size: usize,
    align: usize,
    drop_fn: Option<(fn(*mut u8))>,
    eq_fn: Option<fn(*mut u8, *mut u8) -> bool>,
}

/// Stores metadata describing the type of a component.
#[derive(Copy, Clone)]
pub struct ComponentMeta {
    size: usize,
    align: usize,
    drop_fn: Option<(fn(*mut u8))>,
}

/// Describes the layout of an archetype, including what components
/// and tags shall be attached to entities stored within an archetype.
#[derive(Default)]
pub struct ArchetypeDescription {
    tags: Vec<(TagTypeId, TagMeta)>,
    components: Vec<(ComponentTypeId, ComponentMeta)>,
}

impl ArchetypeDescription {
    /// Adds a tag to the description.
    pub fn register_tag_raw(&mut self, type_id: TagTypeId, type_meta: TagMeta) {
        self.tags.push((type_id, type_meta));
    }

    /// Adds a tag to the description.
    pub fn register_tag<T: Tag>(&mut self) {
        unsafe {
            self.register_tag_raw(
                TagTypeId(TypeId::of::<T>()),
                TagMeta {
                    size: size_of::<T>(),
                    align: std::mem::align_of::<T>(),
                    drop_fn: Some(|ptr| std::ptr::drop_in_place(ptr as *mut T)),
                    eq_fn: Some(|a, b| *(a as *const T) == *(b as *const T)),
                },
            );
        }
    }

    /// Adds a component to the description.
    pub fn register_component_raw(&mut self, type_id: ComponentTypeId, type_meta: ComponentMeta) {
        self.components.push((type_id, type_meta));
    }

    /// Adds a component to the description.
    pub fn register_component<T: Component>(&mut self) {
        unsafe {
            self.register_component_raw(
                ComponentTypeId(TypeId::of::<T>()),
                ComponentMeta {
                    size: size_of::<T>(),
                    align: std::mem::align_of::<T>(),
                    drop_fn: Some(|ptr| std::ptr::drop_in_place(ptr as *mut T)),
                },
            );
        }
    }
}

const MAX_CHUNK_SIZE: usize = 16 * 1024;
const COMPONENT_STORAGE_ALIGNMENT: usize = 64;

/// Unique ID of an archetype.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArchetypeId(usize);

impl ArchetypeId {
    pub(crate) fn new(index: usize) -> Self { ArchetypeId(index) }

    fn index(self) -> usize { self.0 }
}

/// Contains all of the tags attached to the entities in each chunk.
pub struct Tags(SmallVec<[(TagTypeId, TagStorage); 3]>);

impl Tags {
    fn new(mut data: SmallVec<[(TagTypeId, TagStorage); 3]>) -> Self {
        data.sort_by_key(|(t, _)| *t);
        Self(data)
    }

    /// Gets the set of tag values of the specified type attached to all chunks.
    #[inline]
    pub fn get(&self, type_id: TagTypeId) -> Option<&TagStorage> {
        self.0
            .binary_search_by_key(&type_id, |(t, _)| *t)
            .ok()
            .map(|i| unsafe { &self.0.get_unchecked(i).1 })
    }

    /// Mutably gets the set of all tag values of the specified type attached to all chunks.
    #[inline]
    pub fn get_mut(&mut self, type_id: TagTypeId) -> Option<&mut TagStorage> {
        self.0
            .binary_search_by_key(&type_id, |(t, _)| *t)
            .ok()
            .map(move |i| unsafe { &mut self.0.get_unchecked_mut(i).1 })
    }
}

/// Stores entity data in chunks. All entities within an archetype have the same data layout
/// (component and tag types).
pub struct ArchetypeData {
    id: ArchetypeId,
    tags: Tags,
    component_layout: ComponentStorageLayout,
    component_chunks: Vec<ComponentStorage>,
}

impl ArchetypeData {
    fn new(id: ArchetypeId, desc: &ArchetypeDescription) -> Self {
        // create tag storage
        let tags = desc
            .tags
            .iter()
            .map(|(type_id, meta)| (*type_id, TagStorage::new(*meta)))
            .collect();

        // create component data layout
        let max_component_size = desc
            .components
            .iter()
            .map(|(_, meta)| meta.size)
            .max()
            .unwrap_or(0);
        let entity_capacity = std::cmp::max(
            1,
            MAX_CHUNK_SIZE / std::cmp::max(max_component_size, size_of::<Entity>()),
        );
        let mut data_capacity = 0usize;
        let mut component_data_offsets = Vec::new();
        for (type_id, meta) in desc.components.iter() {
            data_capacity = align_up(
                align_up(data_capacity, COMPONENT_STORAGE_ALIGNMENT),
                meta.align,
            );
            component_data_offsets.push((*type_id, data_capacity, *meta));
            data_capacity += meta.size * entity_capacity;
        }
        let data_alignment =
            std::alloc::Layout::from_size_align(data_capacity, COMPONENT_STORAGE_ALIGNMENT)
                .expect("invalid component data size/alignment");

        ArchetypeData {
            id,
            tags: Tags::new(tags),
            component_layout: ComponentStorageLayout {
                capacity: entity_capacity,
                alloc_layout: data_alignment,
                data_layout: component_data_offsets,
            },
            component_chunks: Vec::new(),
        }
    }

    pub(crate) fn alloc_chunk(&mut self) -> (usize, &mut Tags, &mut ComponentStorage) {
        let chunk = self
            .component_layout
            .alloc_storage(ChunkId(self.id, self.component_chunks.len()));
        self.component_chunks.push(chunk);
        (
            self.component_chunks.len() - 1,
            &mut self.tags,
            self.component_chunks.last_mut().unwrap(),
        )
    }

    /// Gets the number of chunks stored within this archetype.
    pub fn len(&self) -> usize { self.component_chunks.len() }

    /// Determines whether this archetype has any chunks.
    pub fn is_empty(&self) -> bool { self.len() < 1 }

    /// Gets the tag storage for the specified tag type.
    pub fn tags(&self, tag_type: TagTypeId) -> Option<&TagStorage> { self.tags.get(tag_type) }

    /// Iterates though all chunks.
    pub fn iter_component_chunks(&self) -> std::slice::Iter<ComponentStorage> {
        self.component_chunks.iter()
    }

    /// Gets a reference to the specified chunk.
    pub fn component_chunk(&self, chunk: usize) -> Option<&ComponentStorage> {
        self.component_chunks.get(chunk)
    }

    /// Gets a mutable reference to the specified chunk.
    pub fn component_chunk_mut(&mut self, chunk: usize) -> Option<&mut ComponentStorage> {
        self.component_chunks.get_mut(chunk)
    }
}

fn align_up(addr: usize, align: usize) -> usize { (addr + (align - 1)) & align.wrapping_neg() }

/// Describes the data layout for a chunk.
struct ComponentStorageLayout {
    capacity: usize,
    alloc_layout: std::alloc::Layout,
    data_layout: Vec<(ComponentTypeId, usize, ComponentMeta)>,
}

impl ComponentStorageLayout {
    fn alloc_storage(&self, id: ChunkId) -> ComponentStorage {
        unsafe {
            let data_storage = std::alloc::alloc(self.alloc_layout);
            let storage_info = self
                .data_layout
                .iter()
                .map(|(ty, offset, meta)| {
                    (
                        *ty,
                        ComponentAccessor {
                            ptr: AtomicRefCell::new(NonNull::new_unchecked(
                                data_storage.add(*offset),
                            )),
                            capacity: self.capacity,
                            count: UnsafeCell::new(0),
                            element_size: meta.size,
                            drop_fn: meta.drop_fn,
                            version: UnsafeCell::new(Wrapping(0)),
                        },
                    )
                })
                .collect();

            ComponentStorage {
                id,
                capacity: self.capacity,
                entities: Vec::with_capacity(self.capacity),
                component_layout: self.alloc_layout,
                component_info: UnsafeCell::new(Components::new(storage_info)),
                component_data: NonNull::new_unchecked(data_storage),
            }
        }
    }
}

/// Unique ID of a chunk.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ChunkId(ArchetypeId, usize);

impl ChunkId {
    pub(crate) fn new(archetype: ArchetypeId, index: usize) -> Self { ChunkId(archetype, index) }

    pub fn archetype_id(&self) -> ArchetypeId { self.0 }

    pub fn index(&self) -> usize { self.1 }
}

/// A set of component slices located on a chunk.
pub struct Components(SmallVec<[(ComponentTypeId, ComponentAccessor); 5]>);

impl Components {
    pub(crate) fn new(mut data: SmallVec<[(ComponentTypeId, ComponentAccessor); 5]>) -> Self {
        data.sort_by_key(|(t, _)| *t);
        Self(data)
    }

    /// Gets a component slice accessor for the specified component type.
    #[inline]
    pub fn get(&self, type_id: ComponentTypeId) -> Option<&ComponentAccessor> {
        self.0
            .binary_search_by_key(&type_id, |(t, _)| *t)
            .ok()
            .map(|i| unsafe { &self.0.get_unchecked(i).1 })
    }

    /// Gets a mutable component slice accessor for the specified component type.
    #[inline]
    pub fn get_mut(&mut self, type_id: ComponentTypeId) -> Option<&mut ComponentAccessor> {
        self.0
            .binary_search_by_key(&type_id, |(t, _)| *t)
            .ok()
            .map(move |i| unsafe { &mut self.0.get_unchecked_mut(i).1 })
    }

    fn iter(&mut self) -> Iter<(ComponentTypeId, ComponentAccessor)> { self.0.iter() }

    fn iter_mut(&mut self) -> IterMut<(ComponentTypeId, ComponentAccessor)> { self.0.iter_mut() }

    fn drain(&mut self) -> Drain<(ComponentTypeId, ComponentAccessor)> { self.0.drain() }
}

/// Stores a chunk of entities and their component data of a specific data layout.
pub struct ComponentStorage {
    id: ChunkId,
    capacity: usize,
    entities: Vec<Entity>,
    component_layout: std::alloc::Layout,
    component_info: UnsafeCell<Components>,
    component_data: NonNull<u8>,
}

impl ComponentStorage {
    /// Gets the unique ID of the chunk.
    pub fn id(&self) -> ChunkId { self.id }

    /// Gets the number of entities stored in the chunk.
    pub fn len(&self) -> usize { self.entities.len() }

    /// Gets the maximum number of entities that can be stored in the chunk.
    pub fn capacity(&self) -> usize { self.capacity }

    /// Determines if the chunk is full.
    pub fn is_full(&self) -> bool { self.len() >= self.capacity }

    /// Determines if the chunk is empty.
    pub fn is_empty(&self) -> bool { self.entities.len() < 1 }

    /// Gets a slice reference containing the IDs of all entities stored in the chunk.
    pub fn entities(&self) -> &[Entity] { self.entities.as_slice() }

    /// Gets a component accessor for the specified component type.
    pub fn components(&self, component_type: ComponentTypeId) -> Option<&ComponentAccessor> {
        unsafe { &*self.component_info.get() }.get(component_type)
    }

    /// Removes an entity from the chunk by swapping it with the last entry.ComponentStorage
    ///
    /// Returns the ID of the entity which was swapped into the removed entity's position.
    pub fn swap_remove(&mut self, index: usize) -> Option<Entity> {
        self.entities.swap_remove(index);
        for (_, component) in unsafe { &mut *self.component_info.get() }.iter_mut() {
            component.writer().swap_remove(index);
        }

        if self.entities.len() > index {
            Some(*self.entities.get(index).unwrap())
        } else {
            None
        }
    }

    /// Gets mutable references to the internal data of the chunk.
    pub fn write(&mut self) -> (&mut Vec<Entity>, &UnsafeCell<Components>) {
        (&mut self.entities, &self.component_info)
    }
}

unsafe impl Sync for ComponentStorage {}

unsafe impl Send for ComponentStorage {}

impl Drop for ComponentStorage {
    fn drop(&mut self) {
        // run the drop functions of all components
        for (_, info) in unsafe { &mut *self.component_info.get() }.drain() {
            if let Some(drop_fn) = info.drop_fn {
                let ptr = info.ptr.get_mut().as_ptr();
                for i in 0..self.len() {
                    unsafe {
                        drop_fn(ptr.add(info.element_size * i));
                    }
                }
            }
        }

        // free the chunk's memory
        unsafe {
            std::alloc::dealloc(self.component_data.as_ptr(), self.component_layout);
        }
    }
}

/// Provides raw access to component data slices.
#[repr(align(64))]
pub struct ComponentAccessor {
    ptr: AtomicRefCell<NonNull<u8>>,
    element_size: usize,
    count: UnsafeCell<usize>,
    capacity: usize,
    drop_fn: Option<fn(*mut u8)>,
    version: UnsafeCell<Wrapping<usize>>,
}

impl ComponentAccessor {
    /// Gets the version of the component slice.
    pub fn version(&self) -> usize { unsafe { (*self.version.get()).0 } }

    /// Gets a raw pointer to the start of the component slice.
    ///
    /// Returns a tuple containing `(pointer, element_size, count)`.
    ///
    /// # Safety
    ///
    /// Access to the component data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub fn data_raw(&self) -> (Ref<Shared, NonNull<u8>>, usize, usize) {
        (self.ptr.get(), self.element_size, unsafe {
            *self.count.get()
        })
    }

    /// Gets a raw pointer to the start of the component slice.
    ///
    /// Returns a tuple containing `(pointer, element_size, count)`.
    ///
    /// # Safety
    ///
    /// Access to the component data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub fn data_raw_mut(&self) -> (RefMut<Exclusive, NonNull<u8>>, usize, usize) {
        // this version increment is not thread safe
        // - but the pointer `get_mut` ensures exclusive access at runtime
        let ptr = self.ptr.get_mut();
        unsafe { *self.version.get() += Wrapping(1) };
        (ptr, self.element_size, unsafe { *self.count.get() })
    }

    /// Gets a shared reference to the slice of components.
    ///
    /// # Safety
    ///
    /// Ensure that `T` is representative of the component data actually stored.
    ///
    /// Access to the component data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub unsafe fn data_slice<T>(&self) -> RefMap<Shared, &[T]> {
        let (ptr, _size, count) = self.data_raw();
        ptr.map_into(|ptr| std::slice::from_raw_parts(ptr.as_ptr() as *const T, count))
    }

    /// Gets a mutable reference to the slice of components.
    ///
    /// # Safety
    ///
    /// Ensure that `T` is representative of the component data actually stored.
    ///
    /// Access to the component data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub unsafe fn data_slice_mut<T>(&self) -> RefMapMut<Exclusive, &mut [T]> {
        let (ptr, _size, count) = self.data_raw_mut();
        ptr.map_into(|ptr| std::slice::from_raw_parts_mut(ptr.as_ptr() as *mut T, count))
    }

    /// Creates a writer for pushing components into or removing from the vec.
    pub fn writer(&mut self) -> ComponentWriter { ComponentWriter::new(self) }
}

impl Debug for ComponentAccessor {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "ComponentAccessor {{ element_size: {}, count: {}, capacity: {}, version: {} }}",
            self.element_size,
            unsafe { *self.count.get() },
            self.capacity,
            self.version()
        )
    }
}

/// Provides methods adding or removing components from a component vec.
pub struct ComponentWriter<'a> {
    accessor: &'a ComponentAccessor,
    ptr: RefMut<'a, Exclusive<'a>, NonNull<u8>>,
}

impl<'a> ComponentWriter<'a> {
    fn new(accessor: &'a ComponentAccessor) -> ComponentWriter<'a> {
        Self {
            accessor,
            ptr: accessor.ptr.get_mut(),
        }
    }

    /// Pushes new components onto the end of the vec.
    ///
    /// # Safety
    ///
    /// Ensure the components pointed to by `components` are representative
    /// of the component types stored in the vec.
    pub unsafe fn push_raw(&mut self, components: NonNull<u8>, count: usize) {
        assert!((*self.accessor.count.get() + count) <= self.accessor.capacity);
        std::ptr::copy_nonoverlapping(
            components.as_ptr(),
            self.ptr
                .as_ptr()
                .add(*self.accessor.count.get() * self.accessor.element_size),
            count * self.accessor.element_size,
        );
        *self.accessor.count.get() += count;
        *self.accessor.version.get() += Wrapping(1);
    }

    /// Pushes new components onto the end of the vec.
    ///
    /// # Safety
    ///
    /// Ensure that the type `T` is representative of the component types stored in the vec.
    pub unsafe fn push<T: Component>(&mut self, components: &[T]) {
        self.push_raw(
            NonNull::new_unchecked(components.as_ptr() as *mut u8),
            components.len(),
        );
    }

    /// Removes the component at the specified index by swapping it with the last component.
    pub fn swap_remove(&mut self, index: usize) {
        unsafe {
            let size = self.accessor.element_size;
            let count = *self.accessor.count.get();
            let to_remove = self.ptr.as_ptr().add(size * index);
            if let Some(drop_fn) = self.accessor.drop_fn {
                drop_fn(to_remove);
            }
            if index < count - 1 {
                let swap_target = self.ptr.as_ptr().add(size * (count - 1));
                std::ptr::copy_nonoverlapping(swap_target, to_remove, size);
            }

            *self.accessor.count.get() -= 1;
        }
    }
}

/// A vector of tag values of a single type.
///
/// Each element in the vector represents the value of tag for
/// the chunk with the corresponding index.
pub struct TagStorage {
    ptr: NonNull<u8>,
    capacity: usize,
    len: usize,
    element: TagMeta,
}

impl TagStorage {
    fn new(element: TagMeta) -> Self {
        let capacity = if element.size == 0 { !0 } else { 4 };

        let ptr = unsafe {
            if element.size > 0 {
                let layout =
                    std::alloc::Layout::from_size_align(capacity * element.size, element.align)
                        .unwrap();
                NonNull::new_unchecked(std::alloc::alloc(layout))
            } else {
                NonNull::new_unchecked(element.align as *mut u8)
            }
        };

        TagStorage {
            ptr,
            capacity,
            len: 0,
            element,
        }
    }

    /// Gets the number of tags contained within the vector.
    pub fn len(&self) -> usize { self.len }

    /// Determines if the vector is empty.
    pub fn is_empty(&self) -> bool { self.len() < 1 }

    /// Pushes a new tag onto the end of the vector.ComponentStorage
    ///
    /// # Safety
    ///
    /// Ensure the tag pointed to by `ptr` is representative
    /// of the tag types stored in the vec.
    pub unsafe fn push_raw(&mut self, ptr: *const u8) {
        if self.len == self.capacity {
            self.grow();
        }

        if self.element.size > 0 {
            let dst = self.ptr.as_ptr().add(self.len * self.element.size);
            std::ptr::copy_nonoverlapping(ptr, dst, self.element.size);
        }

        self.len += 1;
    }

    /// Pushes a new tag onto the end of the vector.ComponentStorage
    ///
    /// # Safety
    ///
    /// Ensure that the type `T` is representative of the tag type stored in the vec.
    pub unsafe fn push<T: Tag>(&mut self, value: T) {
        debug_assert!(
            size_of::<T>() == self.element.size,
            "incompatible element data size"
        );
        self.push_raw(&value as *const T as *const u8);
    }

    /// Gets a raw pointer to the start of the tag slice.
    ///
    /// Returns a tuple containing `(pointer, element_size, count)`.
    ///
    /// # Safety
    ///
    /// Access to the tag data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub unsafe fn data_raw(&self) -> (NonNull<u8>, usize, usize) {
        (self.ptr, self.element.size, self.len)
    }

    /// Gets a shared reference to the slice of tags.
    ///
    /// # Safety
    ///
    /// Ensure that `T` is representative of the tag data actually stored.
    ///
    /// Access to the tag data within the slice is runtime borrow checked.
    /// This call will panic if borrowing rules are broken.
    pub unsafe fn data_slice<T>(&self) -> &[T] {
        debug_assert!(
            size_of::<T>() == self.element.size,
            "incompatible element data size"
        );
        std::slice::from_raw_parts(self.ptr.as_ptr() as *const T, self.len)
    }

    fn grow(&mut self) {
        assert!(self.element.size != 0, "capacity overflow");
        unsafe {
            let (new_cap, ptr) = {
                let layout = std::alloc::Layout::from_size_align(
                    self.capacity * self.element.size,
                    self.element.align,
                )
                .unwrap();
                let new_cap = 2 * self.capacity;
                let ptr =
                    std::alloc::realloc(self.ptr.as_ptr(), layout, new_cap * self.element.size);

                (new_cap, ptr)
            };

            if ptr.is_null() {
                println!("out of memory");
                std::process::abort()
            }

            self.ptr = NonNull::new_unchecked(ptr);
            self.capacity = new_cap;
        }
    }
}

unsafe impl Sync for TagStorage {}

unsafe impl Send for TagStorage {}

impl Drop for TagStorage {
    fn drop(&mut self) {
        if self.element.size > 0 {
            let ptr = self.ptr.as_ptr();

            unsafe {
                if let Some(drop_fn) = self.element.drop_fn {
                    for i in 0..self.len {
                        drop_fn(ptr.add(i * self.element.size));
                    }
                }
                let layout = std::alloc::Layout::from_size_align_unchecked(
                    self.element.size * self.capacity,
                    self.element.align,
                );
                std::alloc::dealloc(ptr, layout);
            }
        }
    }
}

impl Debug for TagStorage {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "TagStorage {{ element_size: {}, count: {}, capacity: {} }}",
            self.element.size, self.len, self.capacity
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct ZeroSize;

    #[test]
    pub fn create() {
        let mut archetypes = Storage::default();

        let mut desc = ArchetypeDescription::default();
        desc.register_tag::<usize>();
        desc.register_component::<isize>();

        let (_arch_id, data) = archetypes.alloc_archetype(&desc);
        let (_, tags, components) = data.alloc_chunk();

        unsafe { tags.get_mut(TagTypeId::of::<usize>()).unwrap().push(1isize) };

        let (chunk_entities, chunk_components) = components.write();

        chunk_entities.push(Entity::new(1, Wrapping(0)));
        unsafe {
            (&mut *chunk_components.get())
                .get_mut(ComponentTypeId::of::<isize>())
                .unwrap()
                .writer()
                .push(&[1usize]);
        }
    }

    #[test]
    pub fn read_components() {
        let mut archetypes = Storage::default();

        let mut desc = ArchetypeDescription::default();
        desc.register_component::<isize>();
        desc.register_component::<usize>();
        desc.register_component::<ZeroSize>();

        let (_arch_id, data) = archetypes.alloc_archetype(&desc);
        let (_, _, components) = data.alloc_chunk();

        let entities = [
            (Entity::new(1, Wrapping(0)), 1isize, 1usize, ZeroSize),
            (Entity::new(2, Wrapping(0)), 2isize, 2usize, ZeroSize),
            (Entity::new(3, Wrapping(0)), 3isize, 3usize, ZeroSize),
        ];

        let (chunk_entities, chunk_components) = components.write();
        for (entity, c1, c2, c3) in entities.iter() {
            chunk_entities.push(*entity);
            unsafe {
                (&mut *chunk_components.get())
                    .get_mut(ComponentTypeId::of::<isize>())
                    .unwrap()
                    .writer()
                    .push(&[*c1]);
                (&mut *chunk_components.get())
                    .get_mut(ComponentTypeId::of::<usize>())
                    .unwrap()
                    .writer()
                    .push(&[*c2]);
                (&mut *chunk_components.get())
                    .get_mut(ComponentTypeId::of::<ZeroSize>())
                    .unwrap()
                    .writer()
                    .push(&[*c3]);
            }
        }

        unsafe {
            for (i, c) in (*chunk_components.get())
                .get(ComponentTypeId::of::<isize>())
                .unwrap()
                .data_slice::<isize>()
                .iter()
                .enumerate()
            {
                assert_eq!(entities[i].1, *c);
            }

            for (i, c) in (*chunk_components.get())
                .get(ComponentTypeId::of::<usize>())
                .unwrap()
                .data_slice::<usize>()
                .iter()
                .enumerate()
            {
                assert_eq!(entities[i].2, *c);
            }

            for (i, c) in (*chunk_components.get())
                .get(ComponentTypeId::of::<ZeroSize>())
                .unwrap()
                .data_slice::<ZeroSize>()
                .iter()
                .enumerate()
            {
                assert_eq!(entities[i].3, *c);
            }
        }
    }

    #[test]
    pub fn read_tags() {
        let mut archetypes = Storage::default();

        let mut desc = ArchetypeDescription::default();
        desc.register_tag::<isize>();
        desc.register_tag::<ZeroSize>();

        let (_arch_id, data) = archetypes.alloc_archetype(&desc);

        let tag_values = [(0isize, ZeroSize), (1isize, ZeroSize), (2isize, ZeroSize)];

        for (t1, t2) in tag_values.iter() {
            let (_, tags, _) = data.alloc_chunk();
            unsafe { tags.get_mut(TagTypeId::of::<isize>()).unwrap().push(*t1) };
            unsafe { tags.get_mut(TagTypeId::of::<ZeroSize>()).unwrap().push(*t2) };
        }

        unsafe {
            let tags1 = data
                .tags(TagTypeId::of::<isize>())
                .unwrap()
                .data_slice::<isize>();
            assert_eq!(tags1.len(), tag_values.len());
            for (i, t) in tags1.iter().enumerate() {
                assert_eq!(tag_values[i].0, *t);
            }

            let tags2 = data
                .tags(TagTypeId::of::<ZeroSize>())
                .unwrap()
                .data_slice::<ZeroSize>();
            assert_eq!(tags2.len(), tag_values.len());
            for (i, t) in tags2.iter().enumerate() {
                assert_eq!(tag_values[i].1, *t);
            }
        }
    }

    #[test]
    pub fn create_zero_size_tags() {
        let mut archetypes = Storage::default();

        let mut desc = ArchetypeDescription::default();
        desc.register_tag::<ZeroSize>();
        desc.register_component::<isize>();

        let (_arch_id, data) = archetypes.alloc_archetype(&desc);
        let (_, tags, components) = data.alloc_chunk();

        unsafe {
            tags.get_mut(TagTypeId::of::<ZeroSize>())
                .unwrap()
                .push(ZeroSize)
        };

        let (chunk_entities, chunk_components) = components.write();

        chunk_entities.push(Entity::new(1, Wrapping(0)));
        unsafe {
            (&mut *chunk_components.get())
                .get_mut(ComponentTypeId::of::<isize>())
                .unwrap()
                .writer()
                .push(&[1usize]);
        }
    }

    #[test]
    pub fn create_zero_size_components() {
        let mut archetypes = Storage::default();

        let mut desc = ArchetypeDescription::default();
        desc.register_tag::<usize>();
        desc.register_component::<ZeroSize>();

        let (_arch_id, data) = archetypes.alloc_archetype(&desc);
        let (_, tags, components) = data.alloc_chunk();

        unsafe { tags.get_mut(TagTypeId::of::<usize>()).unwrap().push(1isize) };

        let (chunk_entities, chunk_components) = components.write();

        chunk_entities.push(Entity::new(1, Wrapping(0)));
        unsafe {
            (&mut *chunk_components.get())
                .get_mut(ComponentTypeId::of::<ZeroSize>())
                .unwrap()
                .writer()
                .push(&[ZeroSize]);
        }
    }
}
