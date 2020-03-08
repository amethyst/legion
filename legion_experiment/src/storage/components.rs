use crate::borrow::{AtomicRefCell, RefMap, RefMapMut};
use crate::entity::Entity;
use crate::storage::archetype::EntityTypeLayout;
use crate::storage::index::ComponentIndex;
use core::any::TypeId;
use smallvec::SmallVec;
use std::mem::size_of;
use std::ptr::NonNull;
use std::slice::Iter;
use std::sync::atomic::{AtomicU64, Ordering};
use std::vec::Drain;

const MAX_CHUNK_SIZE: usize = 16 * 1024 * 10;
const COMPONENT_STORAGE_ALIGNMENT: usize = 64;

/// A unique identifier for an entity component type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
#[cfg(not(feature = "ffi"))]
pub struct ComponentTypeId(pub TypeId);

#[cfg(not(feature = "ffi"))]
impl ComponentTypeId {
    /// Gets the component type ID that represents type `T`.
    pub fn of<T: Component>() -> Self { Self(TypeId::of::<T>()) }
}

/// A unique identifier for an entity component type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
#[cfg(feature = "ffi")]
pub struct ComponentTypeId(pub TypeId, pub u32);

#[cfg(feature = "ffi")]
impl ComponentTypeId {
    /// Gets the component type ID that represents type `T`.
    pub fn of<T: Component>() -> Self { Self(TypeId::of::<T>(), 0) }
}

/// A `Component` is per-entity data that can be attached to a single entity.
pub trait Component: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Component for T {}

/// Stores metadata describing the type of a component.
#[derive(Copy, Clone, PartialEq)]
pub struct ComponentMeta {
    size: usize,
    align: usize,
    drop_fn: Option<fn(*mut u8)>,
}

impl ComponentMeta {
    /// Gets the component meta of component type `T`.
    pub fn of<T: Component>() -> Self {
        ComponentMeta {
            size: size_of::<T>(),
            align: std::mem::align_of::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(|ptr| unsafe { std::ptr::drop_in_place(ptr as *mut T) })
            } else {
                None
            },
        }
    }
}

/// Precomputed component storage memory layout data.
#[derive(Clone)]
pub struct ComponentStorageLayout {
    capacity: usize,
    alloc_layout: std::alloc::Layout,
    data_layout: Vec<(ComponentTypeId, usize, ComponentMeta)>,
}

impl ComponentStorageLayout {
    /// Creates a new storage layout.
    pub fn new(layout: &EntityTypeLayout) -> Self {
        let max_component_size = layout
            .components()
            .map(|(_, meta)| meta.size)
            .max()
            .unwrap_or(0);
        let entity_capacity = std::cmp::max(
            1,
            MAX_CHUNK_SIZE / std::cmp::max(max_component_size, size_of::<Entity>()),
        );
        let mut data_capacity = 0usize;
        let mut component_data_offsets = Vec::new();
        for (type_id, meta) in layout.components() {
            data_capacity = Self::align_up(
                Self::align_up(data_capacity, COMPONENT_STORAGE_ALIGNMENT),
                meta.align,
            );
            component_data_offsets.push((*type_id, data_capacity, *meta));
            data_capacity += meta.size * entity_capacity;
        }
        let mem_layout =
            std::alloc::Layout::from_size_align(data_capacity, COMPONENT_STORAGE_ALIGNMENT)
                .expect("invalid component data size/alignment");

        Self {
            capacity: entity_capacity,
            alloc_layout: mem_layout,
            data_layout: component_data_offsets,
        }
    }

    pub fn capacity(&self) -> usize { self.capacity }

    fn align_up(addr: usize, align: usize) -> usize { (addr + (align - 1)) & align.wrapping_neg() }

    /// Creates a new component storage.
    pub fn alloc_storage(&self) -> ComponentStorage {
        unsafe {
            let ptr = std::alloc::alloc(self.alloc_layout);
            let version = next_version();
            let storage_info = self
                .data_layout
                .iter()
                .map(|&(ty, offset, meta)| {
                    (
                        ty,
                        ComponentVec {
                            data: AtomicRefCell::new((
                                NonNull::new_unchecked(ptr.add(offset)),
                                version,
                            )),
                            capacity: self.capacity,
                            len: 0,
                            meta,
                        },
                    )
                })
                .collect();

            ComponentStorage::new(
                self.capacity,
                self.alloc_layout,
                NonNull::new_unchecked(ptr),
                storage_info,
            )
        }
    }
}

/// Stores vectors of components for entities with the same component layout.
pub struct ComponentStorage {
    /// Maximum number of components (of each type) that can be stored.
    capacity: usize,
    /// Layout of the memory allocation.
    mem_layout: std::alloc::Layout,
    /// Pointer to the block of memory allocated for this chunk.
    /// Each `ComponentVec` owns a region of this block.
    /// We hold onto the pointer here so that we can free the memory on drop.
    mem_ptr: NonNull<u8>,
    /// Vectors of each type of component stored in the chunk.
    slices: SmallVec<[(ComponentTypeId, ComponentVec); 5]>,
}

impl ComponentStorage {
    pub fn new(
        capacity: usize,
        mem_layout: std::alloc::Layout,
        mem_ptr: NonNull<u8>,
        mut slices: SmallVec<[(ComponentTypeId, ComponentVec); 5]>,
    ) -> Self {
        slices.sort_by_key(|&(t, _)| t);
        Self {
            capacity,
            mem_layout,
            mem_ptr,
            slices,
        }
    }

    pub fn iter(&self) -> Iter<(ComponentTypeId, ComponentVec)> { self.slices.iter() }

    pub fn get(&self, type_id: ComponentTypeId) -> Option<&ComponentVec> {
        self.slices
            .binary_search_by_key(&type_id, |&(t, _)| t)
            .ok()
            .map(|i| &self.slices[i].1)
    }

    // pub(crate) fn get_mut(&mut self, type_id: ComponentTypeId) -> Option<&mut ComponentVec> {
    //     self.slices
    //         .binary_search_by_key(&type_id, |&(t, _)| t)
    //         .ok()
    //         .map(move |i| &mut self.slices[i].1)
    // }

    pub(crate) fn swap_remove(&mut self, index: ComponentIndex, drop: bool) {
        for (_, slice) in self.slices.iter_mut() {
            slice.swap_remove(index, drop);
        }
    }

    pub fn writer(&mut self) -> ComponentStorageWriter {
        ComponentStorageWriter {
            vecs: self.slices.iter_mut().map(|(t, v)| (*t, v)).collect(),
        }
    }

    pub(crate) fn clear(&mut self) {
        for (_, slice) in &mut self.slices {
            slice.clear();
        }
    }
}

unsafe impl Send for ComponentStorage {}

unsafe impl Sync for ComponentStorage {}

impl Drop for ComponentStorage {
    fn drop(&mut self) {
        // clear the component vec to drop its components
        // we do this rather than letting the storage's Drop run because we need to
        // make sure all components are dropped before we delete the memory (rust runs this drop before fields)
        self.clear();

        // free the chunk's memory
        unsafe {
            std::alloc::dealloc(self.mem_ptr.as_ptr(), self.mem_layout);
        }
    }
}

pub struct ComponentStorageWriter<'a> {
    vecs: Vec<(ComponentTypeId, &'a mut ComponentVec)>,
}

impl<'a> ComponentStorageWriter<'a> {
    /// Pulls a mutable component vec reference out of the writer.
    pub fn claim(&mut self, type_id: ComponentTypeId) -> Option<&'a mut ComponentVec> {
        if let Some(index) = self.vecs.iter().position(|(t, _)| *t == type_id) {
            let (_, vec) = self.vecs.swap_remove(index);
            Some(vec)
        } else {
            None
        }
    }

    pub fn drain(&mut self) -> Drain<(ComponentTypeId, &'a mut ComponentVec)> {
        self.vecs.drain(..)
    }

    pub fn validate(&mut self) {
        if !self.vecs.is_empty() {
            panic!("not all components in chunk claimed - chunk will be left in an unsound state");
        }
    }
}

/// Component slice version. This is incremented each time the slice is accessed mutably.
pub type Version = u64;

static VERSION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_version() -> u64 {
    VERSION_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .checked_add(1)
        .unwrap()
}

/// Provides access to a slice of components.
pub struct ComponentVec {
    data: AtomicRefCell<(NonNull<u8>, Version)>,
    capacity: usize,
    len: usize,
    meta: ComponentMeta,
}

impl ComponentVec {
    /// Gets the version of the component slice.
    pub fn version(&self) -> Version { self.data.get().1 }

    /// Gets a raw pointer to the beginning of the slice.
    pub fn ptr(&self) -> RefMap<*const u8> {
        self.data
            .get()
            .map_into(|(ptr, _)| ptr.as_ptr() as *const u8)
    }

    /// Gets a mutable raw pointer to the beginning of the slice. Increments the component slice version.
    ///
    /// # Safety
    ///
    /// Ensure the contents of the slice are not aliased via any other calls
    /// to `get`, `get_mut`, `get_slice` or `get_slice_mut`.
    pub unsafe fn ptr_mut(&self) -> RefMapMut<*mut u8> {
        let mut data = self.data.get_mut();
        data.1 = next_version();
        data.map_into(|(ptr, _)| ptr.as_ptr())
    }

    /// Gets the maximum capacity of this storage vector.
    pub fn capacity(&self) -> usize { self.capacity }

    /// Gets the current stored component count.
    pub fn len(&self) -> usize { self.len }

    /// Determines if the component storage is empty.
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Gets the size of each stored component element in bytes.
    pub fn element_size(&self) -> usize { self.meta.size }

    /// Interprets the contents of this slice as a `&[T]`.
    ///
    /// # Safety
    ///
    /// Ensure that the type `T` is a valid interpretation of the component type
    /// stored.
    pub unsafe fn get_slice<T: Component>(&self) -> RefMap<&[T]> {
        self.ptr()
            .map_into(|ptr| std::slice::from_raw_parts(ptr as *const _ as *const T, self.len))
    }

    /// Interprets the contents of this slice as a `&mut [T]`. Increments the component slice version.
    ///
    /// # Safety
    ///
    /// Ensure the contents of the slice are not aliased via any other calls
    /// to `get`, `get_mut`, `get_slice` or `get_slice_mut`.
    ///
    /// Ensure that the type `T` is a valid interpretation of the component type
    /// stored.
    pub unsafe fn get_slice_mut<T: Component>(&self) -> RefMapMut<&mut [T]> {
        self.ptr_mut()
            .map_into(|ptr| std::slice::from_raw_parts_mut(ptr as *mut _ as *mut T, self.len))
    }

    /// Increases the length of the associated `ComponentSlice` by `count`
    /// and returns a pointer to the start of uninitialized memory that is reserved as a result.
    /// Increments the component slice version.
    ///
    /// # Safety
    ///
    /// Ensure the memory returned by this function is properly initialized before calling
    /// any other storage function. Ensure that the data written into the returned pointer
    /// is representative of the component types stored in the associated ComponentResourceSet.
    ///
    /// # Panics
    ///
    /// Will panic when an internal u64 counter overflows.
    /// It will happen in 50000 years if you do 10000 mutations a millisecond.
    pub unsafe fn reserve_raw(&mut self, count: usize) -> NonNull<u8> {
        debug_assert!((self.len + count) <= self.capacity);
        let (ptr, version) = self.data.inner_mut();
        let ptr = NonNull::new_unchecked(ptr.as_ptr().add(self.len * self.meta.size));
        self.len += count;
        *version = next_version();
        ptr
    }

    /// Pushes new components onto the end of the vec. Increments the component slice version.
    ///
    /// # Safety
    ///
    /// Ensure the components pointed to by `components` are representative
    /// of the component types stored in the vec.
    ///
    /// This function will _copy_ all elements into the chunk. If the source is not `Copy`,
    /// the caller must then `mem::forget` the source such that the destructor does not run
    /// on the original data.
    ///
    /// # Panics
    ///
    /// Will panic when an internal u64 counter overflows.
    /// It will happen in 50000 years if you do 10000 mutations a millisecond.
    pub unsafe fn push_raw(&mut self, components: NonNull<u8>, count: usize) {
        debug_assert!((self.len + count) <= self.capacity);
        {
            let ptr = self.ptr_mut();
            std::ptr::copy_nonoverlapping(
                components.as_ptr(),
                ptr.add(self.len * self.meta.size),
                count * self.meta.size,
            );
        }
        self.len += count;
    }

    /// Pushes new components onto the end of the vec. Increments the component slice version.
    ///
    /// # Safety
    ///
    /// Ensure that the type `T` is representative of the component types stored in the vec.
    ///
    /// This function will _copy_ all elements of `T` into the chunk. If `T` is not `Copy`,
    /// the caller must then `mem::forget` the source such that the destructor does not run
    /// on the original data.
    pub unsafe fn push<T: Component>(&mut self, components: &[T]) {
        self.push_raw(
            NonNull::new_unchecked(components.as_ptr() as *mut u8),
            components.len(),
        );
    }

    /// Removes the component at the specified index by swapping it with the last component.
    fn swap_remove(&mut self, ComponentIndex(index): ComponentIndex, drop: bool) {
        unsafe {
            let size = self.meta.size;
            let ptr = self.data.inner_mut().0.as_ptr();
            let to_remove = ptr.add(size * index);
            if drop {
                if let Some(drop_fn) = self.meta.drop_fn {
                    drop_fn(to_remove);
                }
            }

            let count = self.len;
            if index < count - 1 {
                let swap_target = ptr.add(size * (count - 1));
                std::ptr::copy_nonoverlapping(swap_target, to_remove, size);
            }

            self.len -= 1;
        }
    }

    /// Drops the component stored at `index` without moving any other data or
    /// altering the number of elements.
    ///
    /// # Safety
    ///
    /// Ensure that this function is only ever called once on a given index and that a valid
    /// new value is written into this location before it is next accessed.
    pub(crate) unsafe fn drop_in_place(&mut self, ComponentIndex(index): ComponentIndex) {
        if let Some(drop_fn) = self.meta.drop_fn {
            let size = self.meta.size;
            let (ptr, _) = self.data.inner_mut();
            let to_remove = ptr.as_ptr().add(size * index);
            drop_fn(to_remove);
        }
    }

    /// Clears all components from the vector.
    fn clear(&mut self) {
        if let Some(drop_fn) = self.meta.drop_fn {
            let (data, _) = self.data.inner_mut();
            for i in 0..self.len {
                unsafe {
                    drop_fn(data.as_ptr().add(self.meta.size * i));
                }
            }
        }

        self.len = 0;
    }
}

impl Drop for ComponentVec {
    fn drop(&mut self) { self.clear(); }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::storage::archetype::EntityTypeLayout;

    #[test]
    fn create_storage() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<usize>();
        entity_layout.register_tag::<bool>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let _ = component_layout.alloc_storage();
    }

    #[test]
    fn create_empty_storage() {
        let entity_layout = EntityTypeLayout::default();
        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let storage = component_layout.alloc_storage();

        assert_eq!(storage.iter().count(), 0);
    }

    #[test]
    fn create_zero_sized_component() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<()>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let storage = component_layout.alloc_storage();

        assert!(storage.get(ComponentTypeId::of::<()>()).is_some());
    }

    #[test]
    fn insert_components() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<usize>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let mut storage = component_layout.alloc_storage();
        let components: [usize; 5] = [0, 1, 2, 3, 4];

        {
            let mut writer = storage.writer();
            let vec = writer.claim(ComponentTypeId::of::<usize>()).unwrap();
            unsafe { vec.push(&components) };
        }

        let vec = storage.get(ComponentTypeId::of::<usize>()).unwrap();
        assert_eq!(vec.len(), 5);

        let slice = unsafe { vec.get_slice::<usize>() };
        assert_eq!(slice.len(), 5);

        for (i, c) in components.iter().enumerate() {
            assert_eq!(&slice[i], c);
        }
    }

    #[test]
    fn insert_zero_sized_components() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<()>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let mut storage = component_layout.alloc_storage();
        let components: [(); 5] = [(), (), (), (), ()];

        {
            let mut writer = storage.writer();
            let vec = writer.claim(ComponentTypeId::of::<()>()).unwrap();
            unsafe { vec.push(&components) };
        }

        let vec = storage.get(ComponentTypeId::of::<()>()).unwrap();
        assert_eq!(vec.len(), 5);

        let slice = unsafe { vec.get_slice::<()>() };
        assert_eq!(slice.len(), 5);

        for (i, c) in components.iter().enumerate() {
            assert_eq!(&slice[i], c);
        }
    }

    #[test]
    fn clear_components() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<usize>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let mut storage = component_layout.alloc_storage();
        let components: [usize; 5] = [0, 1, 2, 3, 4];

        {
            let mut writer = storage.writer();
            let vec = writer.claim(ComponentTypeId::of::<usize>()).unwrap();
            unsafe { vec.push(&components) };
        }

        {
            let vec = storage.get(ComponentTypeId::of::<usize>()).unwrap();
            assert_eq!(vec.len(), 5);

            let slice = unsafe { vec.get_slice::<usize>() };
            assert_eq!(slice.len(), 5);
        }

        storage.clear();

        let vec = storage.get(ComponentTypeId::of::<usize>()).unwrap();
        assert_eq!(vec.len(), 0);

        let slice = unsafe { vec.get_slice::<usize>() };
        assert_eq!(slice.len(), 0);
    }

    #[test]
    fn swap_remove_first() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<usize>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let mut storage = component_layout.alloc_storage();
        let components: [usize; 5] = [0, 1, 2, 3, 4];

        {
            let mut writer = storage.writer();
            let vec = writer.claim(ComponentTypeId::of::<usize>()).unwrap();
            unsafe { vec.push(&components) };
        }

        storage.swap_remove(ComponentIndex(0), true);

        let vec = storage.get(ComponentTypeId::of::<usize>()).unwrap();
        assert_eq!(vec.len(), 4);

        let slice = unsafe { vec.get_slice::<usize>() };
        assert_eq!(slice.len(), 4);

        let components: [usize; 4] = [4, 1, 2, 3];
        for (i, c) in components.iter().enumerate() {
            assert_eq!(&slice[i], c);
        }
    }

    #[test]
    fn swap_remove_last() {
        let mut entity_layout = EntityTypeLayout::default();
        entity_layout.register_component::<usize>();

        let component_layout = ComponentStorageLayout::new(&entity_layout);
        let mut storage = component_layout.alloc_storage();
        let components: [usize; 5] = [0, 1, 2, 3, 4];

        {
            let mut writer = storage.writer();
            let vec = writer.claim(ComponentTypeId::of::<usize>()).unwrap();
            unsafe { vec.push(&components) };
        }

        storage.swap_remove(ComponentIndex(4), true);

        let vec = storage.get(ComponentTypeId::of::<usize>()).unwrap();
        assert_eq!(vec.len(), 4);

        let slice = unsafe { vec.get_slice::<usize>() };
        assert_eq!(slice.len(), 4);

        let components: [usize; 4] = [0, 1, 2, 3];
        for (i, c) in components.iter().enumerate() {
            assert_eq!(&slice[i], c);
        }
    }
}
