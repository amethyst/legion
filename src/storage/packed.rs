//! Component storage which can pack archetypes into contiguous memory.

use super::{
    archetype::ArchetypeIndex, component::Component, next_component_version, ComponentIndex,
    ComponentMeta, ComponentSlice, ComponentSliceMut, ComponentStorage, Epoch,
    UnknownComponentStorage,
};
use std::{
    alloc::Layout,
    cell::UnsafeCell,
    iter::Zip,
    mem::{align_of, size_of},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    rc::Rc,
    slice::Iter,
};

/// A memory allocation for an array of `T`.
#[derive(Debug)]
struct RawAlloc<T> {
    ptr: NonNull<T>,
    cap: usize,
}

impl<T> RawAlloc<T> {
    fn new(min_capacity: usize) -> Self {
        if size_of::<T>() == 0 {
            Self {
                ptr: NonNull::dangling(),
                cap: !0,
            }
        } else if min_capacity == 0 {
            Self {
                ptr: NonNull::dangling(),
                cap: 0,
            }
        } else {
            let layout =
                Layout::from_size_align(size_of::<T>() * min_capacity, align_of::<T>()).unwrap();
            Self {
                ptr: NonNull::new(unsafe { std::alloc::alloc(layout) as *mut _ }).unwrap(),
                cap: min_capacity,
            }
        }
    }

    fn layout(&self) -> Layout {
        Layout::from_size_align(size_of::<T>() * self.cap, align_of::<T>()).unwrap()
    }

    fn grow(&mut self, new_capacity: usize) {
        debug_assert!(self.cap < new_capacity);

        unsafe {
            let dst_ptr = if self.cap == 0 {
                let layout =
                    Layout::from_size_align(size_of::<T>() * new_capacity, align_of::<T>())
                        .unwrap();
                std::alloc::alloc(layout) as *mut T
            } else {
                std::alloc::realloc(
                    self.ptr.as_ptr() as *mut u8,
                    self.layout(),
                    size_of::<T>() * new_capacity,
                ) as *mut T
            };
            if let Some(new_ptr) = NonNull::new(dst_ptr) {
                self.ptr = new_ptr;
                self.cap = new_capacity;
            } else {
                std::alloc::handle_alloc_error(Layout::from_size_align_unchecked(
                    size_of::<T>() * new_capacity,
                    align_of::<T>(),
                ));
            }
        }
    }
}

impl<T> Drop for RawAlloc<T> {
    fn drop(&mut self) {
        if size_of::<T>() != 0 && self.cap > 0 {
            unsafe {
                let layout =
                    Layout::from_size_align_unchecked(size_of::<T>() * self.cap, align_of::<T>());
                std::alloc::dealloc(self.ptr.as_ptr() as *mut _, layout);
            }
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)] // it isn't dead - apparent rustc bug
enum ComponentVec<T> {
    Packed {
        raw: Rc<RawAlloc<T>>,
        offset: usize,
        len: usize,
        cap: usize,
    },
    Loose {
        raw: RawAlloc<T>,
        len: usize,
        last_written: Epoch,
    },
}

impl<T> ComponentVec<T> {
    fn new() -> Self {
        Self::Loose {
            raw: RawAlloc::new(0),
            len: 0,
            last_written: 0,
        }
    }

    fn should_pack(&self, epoch_threshold: Epoch) -> bool {
        match self {
            Self::Loose { last_written, .. } => *last_written <= epoch_threshold,
            _ => true,
        }
    }

    fn as_raw_slice(&self) -> (NonNull<T>, usize) {
        match self {
            Self::Packed {
                raw, offset, len, ..
            } => (
                unsafe { NonNull::new_unchecked(raw.ptr.as_ptr().add(*offset)) },
                *len,
            ),
            Self::Loose { raw, len, .. } => {
                (unsafe { NonNull::new_unchecked(raw.ptr.as_ptr()) }, *len)
            }
        }
    }

    fn estimate_fragmentation(&self) -> f32 {
        match self {
            Self::Loose { .. } => 1f32,
            Self::Packed { len, cap, .. } => {
                let empty = cap - len;
                f32::min(1f32, empty as f32 * size_of::<T>() as f32 / 16f32)
            }
        }
    }

    unsafe fn extend_memcopy(&mut self, epoch: Epoch, ptr: *const T, count: usize) {
        self.ensure_capacity(epoch, count);
        let (dst, len) = self.as_raw_slice();
        std::ptr::copy_nonoverlapping(ptr, dst.as_ptr().add(len), count);
        match self {
            Self::Packed { len, .. } => *len += count,
            Self::Loose {
                len, last_written, ..
            } => {
                *len += count;
                *last_written = epoch;
            }
        }
    }

    fn ensure_capacity(&mut self, epoch: Epoch, space: usize) {
        let (cap, len) = match self {
            Self::Packed { cap, len, .. } => (*cap, *len),
            Self::Loose { raw, len, .. } => (raw.cap, *len),
        };

        if cap - len < space {
            self.grow(epoch, len + space);
        }
    }

    fn swap_remove(&mut self, epoch: Epoch, index: usize) -> T {
        let (ptr, len) = self.as_raw_slice();
        assert!(len > index);

        unsafe {
            let item_ptr = ptr.as_ptr().add(index);
            let last_ptr = ptr.as_ptr().add(len - 1);
            if index < len - 1 {
                std::ptr::swap(item_ptr, last_ptr);
            }
            let value = std::ptr::read(last_ptr);
            match self {
                Self::Packed { len, .. } => *len -= 1,
                Self::Loose {
                    len, last_written, ..
                } => {
                    *len -= 1;
                    *last_written = epoch;
                }
            }
            value
        }
    }

    fn grow(&mut self, epoch: Epoch, new_capacity: usize) {
        debug_assert_ne!(std::mem::size_of::<T>(), 0);

        match self {
            Self::Packed {
                raw,
                offset,
                len,
                cap,
            } => {
                // if we are currently packed, then allocate new storage and switch to loose
                debug_assert!(*cap < new_capacity);
                let new_alloc = RawAlloc::new(*len);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        raw.ptr.as_ptr().add(*offset),
                        new_alloc.ptr.as_ptr(),
                        *len,
                    )
                };
                *self = Self::Loose {
                    raw: new_alloc,
                    len: *len,
                    last_written: epoch,
                };
            }
            Self::Loose {
                raw, last_written, ..
            } => {
                // if we are already free, try and resize the allocation
                raw.grow(new_capacity);
                *last_written = epoch;
            }
        };
    }

    unsafe fn pack(&mut self, dst: Rc<RawAlloc<T>>, offset: usize) {
        let (ptr, len) = self.as_raw_slice();
        debug_assert_ne!(std::mem::size_of::<T>(), 0);
        debug_assert!(dst.cap >= offset + len);
        std::ptr::copy_nonoverlapping(ptr.as_ptr(), dst.ptr.as_ptr().add(offset), len);
        *self = Self::Packed {
            raw: dst,
            offset,
            len,
            cap: len,
        }
    }
}

impl<T> Deref for ComponentVec<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        let (ptr, len) = self.as_raw_slice();
        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), len) }
    }
}

impl<T> DerefMut for ComponentVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let (ptr, len) = self.as_raw_slice();
        unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), len) }
    }
}

impl<T> Drop for ComponentVec<T> {
    fn drop(&mut self) {
        if std::mem::needs_drop::<T>() {
            unsafe {
                let (ptr, len) = self.as_raw_slice();
                for i in 0..len {
                    std::ptr::drop_in_place(ptr.as_ptr().add(i));
                }
            }
        }
    }
}

/// Stores a slice of components of type `T` for each archetype.
/// Archetype slices are sorted according to the group that component `T` belongs to.
/// Each slice _may_ be packed into a single allocation to optimise for group-based access.
#[derive(Debug)]
pub struct PackedStorage<T: Component> {
    // Sparse indirection table
    index: Vec<usize>,
    // Ordered archetype slices: (ptr, len)
    slices: Vec<(NonNull<T>, usize)>,
    // The total number of components stored
    entity_len: usize,
    // The current epoch
    epoch: Epoch,
    // Ordered archetype versions
    versions: Vec<UnsafeCell<u64>>,
    // Ordered allocation metadata
    allocations: Vec<ComponentVec<T>>,
}

// these are needed because of the UnsafeCell in versions
// but we write protect that ourselves
unsafe impl<T: Component> Send for PackedStorage<T> {}
unsafe impl<T: Component> Sync for PackedStorage<T> {}

impl<T: Component> PackedStorage<T> {
    fn swap_remove_internal(
        &mut self,
        ArchetypeIndex(archetype): ArchetypeIndex,
        ComponentIndex(index): ComponentIndex,
    ) -> T {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        let component = allocation.swap_remove(self.epoch, index as usize);
        self.update_slice(slice_index);
        self.entity_len -= 1;
        component
    }

    #[inline]
    fn update_slice(&mut self, slice_index: usize) {
        self.slices[slice_index] = self.allocations[slice_index].as_raw_slice();
    }

    fn index(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> usize {
        self.index[archetype as usize]
    }
}

impl<T: Component> UnknownComponentStorage for PackedStorage<T> {
    fn move_component(
        &mut self,
        source: ArchetypeIndex,
        index: ComponentIndex,
        dst: ArchetypeIndex,
    ) {
        // find archetype locations
        let src_slice_index = self.index(source);
        let dst_slice_index = self.index(dst);

        // remove component from source slice
        let src_allocation = &mut self.allocations[src_slice_index];
        let value = src_allocation.swap_remove(self.epoch, index.0 as usize);

        // insert component into destination slice
        let dst_allocation = &mut self.allocations[dst_slice_index];
        unsafe {
            dst_allocation.extend_memcopy(self.epoch, &value as *const _, 1);
            *self.versions[dst_slice_index].get() = next_component_version();
        }

        // update slice pointers
        self.update_slice(src_slice_index);
        self.update_slice(dst_slice_index);

        // forget component to prevent it being dropped (we copied it into the destination)
        std::mem::forget(value);
    }

    fn insert_archetype(&mut self, archetype: ArchetypeIndex, index: Option<usize>) {
        let index = index.unwrap_or_else(|| self.slices.len());
        let arch_index = archetype.0 as usize;

        // create new vector for archetype
        let allocation = ComponentVec::<T>::new();

        // insert archetype into collections
        self.slices.insert(index, allocation.as_raw_slice());
        self.versions.insert(index, UnsafeCell::new(0));
        self.allocations.insert(index, allocation);

        // update index
        for i in self.index.iter_mut().filter(|i| **i != !0 && **i >= index) {
            *i += 1;
        }
        if arch_index >= self.index.len() {
            self.index.resize(arch_index + 1, !0);
        }
        self.index[arch_index] = index;
    }

    fn transfer_archetype(
        &mut self,
        src_archetype: ArchetypeIndex,
        dst_archetype: ArchetypeIndex,
        dst: &mut dyn UnknownComponentStorage,
    ) {
        let dst = dst.downcast_mut::<Self>().unwrap();
        let src_index = self.index(src_archetype);
        let dst_index = dst.index(dst_archetype);

        // update total counts
        let count = self.allocations[src_index].len();
        self.entity_len -= count;
        dst.entity_len += count;

        if dst.allocations[dst_index].len() == 0 {
            // fast path:
            // swap the allocations
            std::mem::swap(
                &mut self.allocations[src_index],
                &mut dst.allocations[dst_index],
            );

            // bump destination version
            unsafe { *dst.versions[dst_index].get() = next_component_version() };
        } else {
            // memcopy components into the destination
            let (ptr, len) = self.get_raw(src_archetype).unwrap();
            unsafe { dst.extend_memcopy_raw(dst_archetype, ptr, len) };

            // clear and forget source
            let mut swapped = ComponentVec::<T>::new();
            std::mem::swap(&mut self.allocations[src_index], &mut swapped);
            std::mem::forget(swapped);
        }

        // update slice pointers
        self.update_slice(src_index);
        dst.update_slice(dst_index);
    }

    /// Moves a component to a new storage.
    fn transfer_component(
        &mut self,
        src_archetype: ArchetypeIndex,
        src_component: ComponentIndex,
        dst_archetype: ArchetypeIndex,
        dst: &mut dyn UnknownComponentStorage,
    ) {
        let component = self.swap_remove_internal(src_archetype, src_component);
        unsafe { dst.extend_memcopy_raw(dst_archetype, &component as *const T as *const u8, 1) };
        std::mem::forget(component);
    }

    fn swap_remove(&mut self, archetype: ArchetypeIndex, index: ComponentIndex) {
        self.swap_remove_internal(archetype, index);
    }

    fn pack(&mut self, age_threshold: Epoch) -> usize {
        if size_of::<T>() == 0 {
            return 0;
        }

        let epoch_threshold = self.epoch - age_threshold;

        let len = self
            .slices
            .iter()
            .zip(self.allocations.iter())
            .filter(|(_, allocation)| allocation.should_pack(epoch_threshold))
            .map(|((_, len), _)| *len)
            .sum();

        unsafe {
            let packed = Rc::new(RawAlloc::new(len));

            let mut cursor = 0;
            for (alloc, slice) in self
                .allocations
                .iter_mut()
                .zip(self.slices.iter_mut())
                .filter(|(allocation, _)| allocation.should_pack(epoch_threshold))
            {
                alloc.pack(packed.clone(), cursor);
                *slice = alloc.as_raw_slice();
                cursor += slice.1;
            }

            cursor
        }
    }

    fn fragmentation(&self) -> f32 {
        self.allocations
            .iter()
            .fold(0f32, |x, y| x + y.estimate_fragmentation())
            / self.entity_len as f32
    }

    fn element_vtable(&self) -> ComponentMeta { ComponentMeta::of::<T>() }

    fn get_raw(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> Option<(*const u8, usize)> {
        let slice_index = *self.index.get(archetype as usize)?;
        let (ptr, len) = self.slices.get(slice_index)?;
        Some((ptr.as_ptr() as *const u8, *len))
    }

    unsafe fn get_mut_raw(
        &self,
        ArchetypeIndex(archetype): ArchetypeIndex,
    ) -> Option<(*mut u8, usize)> {
        let slice_index = *self.index.get(archetype as usize)?;
        let (ptr, len) = self.slices.get(slice_index)?;
        *self.versions.get_unchecked(slice_index).get() = next_component_version();
        Some((ptr.as_ptr() as *mut u8, *len))
    }

    unsafe fn extend_memcopy_raw(
        &mut self,
        ArchetypeIndex(archetype): ArchetypeIndex,
        ptr: *const u8,
        count: usize,
    ) {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        allocation.extend_memcopy(self.epoch, ptr as *const T, count);
        self.slices[slice_index] = allocation.as_raw_slice();
        self.entity_len += count;
        *self.versions[slice_index].get() = next_component_version();
    }

    fn increment_epoch(&mut self) { self.epoch += 1; }
}

impl<T: Component> Default for PackedStorage<T> {
    fn default() -> Self {
        Self {
            index: Vec::new(),
            slices: Vec::new(),
            versions: Vec::new(),
            allocations: Vec::new(),
            entity_len: 0,
            epoch: 0,
        }
    }
}

impl<'a, T: Component> ComponentStorage<'a, T> for PackedStorage<T> {
    type Iter = ComponentIter<'a, T>;
    type IterMut = ComponentIterMut<'a, T>;

    unsafe fn extend_memcopy(&mut self, archetype: ArchetypeIndex, ptr: *const T, count: usize) {
        self.extend_memcopy_raw(archetype, ptr as *const u8, count);
    }

    fn ensure_capacity(&mut self, ArchetypeIndex(archetype): ArchetypeIndex, capacity: usize) {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        allocation.ensure_capacity(self.epoch, capacity);
    }

    fn get(&'a self, ArchetypeIndex(archetype): ArchetypeIndex) -> Option<ComponentSlice<'a, T>> {
        let slice_index = *self.index.get(archetype as usize)?;
        let (ptr, len) = self.slices.get(slice_index)?;
        let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), *len as usize) };
        let version = unsafe { &*self.versions.get_unchecked(slice_index).get() };
        Some(ComponentSlice::new(slice, version))
    }

    unsafe fn get_mut(
        &'a self,
        ArchetypeIndex(archetype): ArchetypeIndex,
    ) -> Option<ComponentSliceMut<'a, T>> {
        let slice_index = *self.index.get(archetype as usize)?;
        let (ptr, len) = self.slices.get(slice_index)?;
        let slice = std::slice::from_raw_parts_mut(ptr.as_ptr(), *len as usize);
        let version = &mut *self.versions.get_unchecked(slice_index).get();
        Some(ComponentSliceMut::new(slice, version))
    }

    fn iter(&'a self, start_inclusive: usize, end_exclusive: usize) -> Self::Iter {
        ComponentIter {
            slices: self.slices[start_inclusive..end_exclusive]
                .iter()
                .zip(self.versions[start_inclusive..end_exclusive].iter()),
        }
    }

    unsafe fn iter_mut(&'a self, start_inclusive: usize, end_exclusive: usize) -> Self::IterMut {
        ComponentIterMut {
            slices: self.slices[start_inclusive..end_exclusive]
                .iter()
                .zip(self.versions[start_inclusive..end_exclusive].iter()),
        }
    }

    fn len(&self) -> usize { self.allocations.len() }
}

#[doc(hidden)]
pub struct ComponentIter<'a, T> {
    slices: Zip<Iter<'a, (NonNull<T>, usize)>, Iter<'a, UnsafeCell<u64>>>,
}

impl<'a, T: Component> Iterator for ComponentIter<'a, T> {
    type Item = ComponentSlice<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.slices.next().map(|((ptr, len), version)| {
            let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), *len as usize) };
            let version = unsafe { &*version.get() };
            ComponentSlice::new(slice, version)
        })
    }
}

#[doc(hidden)]
pub struct ComponentIterMut<'a, T> {
    slices: Zip<Iter<'a, (NonNull<T>, usize)>, Iter<'a, UnsafeCell<u64>>>,
}

impl<'a, T: Component> Iterator for ComponentIterMut<'a, T> {
    type Item = ComponentSliceMut<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.slices.next().map(|((ptr, len), version)| {
            // safety: we know each slice is disjoint
            let slice = unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), *len as usize) };
            let version = unsafe { &mut *version.get() };
            ComponentSliceMut::new(slice, version)
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_storage() { let _ = PackedStorage::<usize>::default(); }

    #[test]
    fn create_zst_storage() { let _ = PackedStorage::<()>::default(); }

    #[test]
    fn insert_archetype() {
        let mut storage = PackedStorage::<usize>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));
    }

    #[test]
    fn insert_zst_archetype() {
        let mut storage = PackedStorage::<()>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));
    }

    #[test]
    fn insert_components() {
        let mut storage = PackedStorage::<usize>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));

        unsafe {
            let components = vec![1, 2, 3];
            let ptr = components.as_ptr();
            storage.extend_memcopy(ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);

            let slice = storage.get_mut(ArchetypeIndex(0)).unwrap();
            assert_eq!(slice.into_slice(), &[1usize, 2usize, 3usize]);
        }
    }

    #[test]
    fn insert_zst_components() {
        let mut storage = PackedStorage::<()>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));

        unsafe {
            let components = vec![(), (), ()];
            let ptr = components.as_ptr();
            storage.extend_memcopy(ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);

            let slice = storage.get_mut(ArchetypeIndex(0)).unwrap();
            assert_eq!(slice.into_slice(), &[(), (), ()]);
        }
    }

    #[test]
    fn swap_remove_first() {
        let mut storage = PackedStorage::<usize>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));

        unsafe {
            let components = vec![1, 2, 3];
            let ptr = components.as_ptr();
            storage.extend_memcopy(ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);
        }

        storage.swap_remove(ArchetypeIndex(0), ComponentIndex(0));

        unsafe {
            let slice = storage.get_mut(ArchetypeIndex(0)).unwrap();
            assert_eq!(slice.into_slice(), &[3usize, 2usize]);
        }
    }

    #[test]
    fn swap_remove_last() {
        let mut storage = PackedStorage::<usize>::default();
        storage.insert_archetype(ArchetypeIndex(0), Some(0));

        unsafe {
            let components = vec![1, 2, 3];
            let ptr = components.as_ptr();
            storage.extend_memcopy(ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);
        }

        storage.swap_remove(ArchetypeIndex(0), ComponentIndex(2));

        unsafe {
            let slice = storage.get_mut(ArchetypeIndex(0)).unwrap();
            assert_eq!(slice.into_slice(), &[1usize, 2usize]);
        }
    }
}
