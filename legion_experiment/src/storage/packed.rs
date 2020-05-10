use super::{
    archetype::ArchetypeIndex, component::Component, ComponentSlice, ComponentSliceMut,
    ComponentStorage, UnknownComponentStorage,
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
    sync::atomic::{AtomicU64, Ordering},
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
                ptr: NonNull::new(std::mem::align_of::<T>() as *mut _).unwrap(),
                cap: !0,
            }
        } else if min_capacity == 0 {
            Self {
                ptr: NonNull::new(std::mem::align_of::<T>() as *mut _).unwrap(),
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
        last_written: u64,
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

    fn should_pack(&self, epoch_threshold: u64) -> bool {
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

    unsafe fn extend_memcopy(&mut self, epoch: u64, ptr: *const T, count: usize) {
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

    fn ensure_capacity(&mut self, epoch: u64, space: usize) {
        let (cap, len) = match self {
            Self::Packed { cap, len, .. } => (*cap, *len),
            Self::Loose { raw, len, .. } => (raw.cap, *len),
        };

        if cap - len < space {
            self.grow(epoch, len + space);
        }
    }

    fn swap_remove(&mut self, epoch: u64, index: usize) -> T {
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

    fn grow(&mut self, epoch: u64, new_capacity: usize) {
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

static COMPONENT_VERSION: AtomicU64 = AtomicU64::new(0);
pub(crate) fn next_component_version() -> u64 { COMPONENT_VERSION.fetch_add(1, Ordering::SeqCst) }

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
    // Ordered archetype versions
    versions: Vec<UnsafeCell<u64>>,
    // Ordered allocation metadata
    allocations: Vec<ComponentVec<T>>,
    // An estimate of the performance lost due to archetype fragmentation
    fragmentation: f32,
}

impl<T: Component> UnknownComponentStorage for PackedStorage<T> {
    fn move_component(
        &mut self,
        epoch: u64,
        ArchetypeIndex(source): ArchetypeIndex,
        index: usize,
        ArchetypeIndex(dst): ArchetypeIndex,
    ) {
        let src_slice_index = self.index[source as usize];
        let dst_slice_index = self.index[dst as usize];
        let src_allocation = &mut self.allocations[src_slice_index];
        let previous_fragmentation = src_allocation.estimate_fragmentation();
        let value = src_allocation.swap_remove(epoch, index);
        self.fragmentation += src_allocation.estimate_fragmentation() - previous_fragmentation;
        unsafe {
            let dst_allocation = &mut self.allocations[dst_slice_index];
            let previous_fragmentation = dst_allocation.estimate_fragmentation();
            dst_allocation.extend_memcopy(epoch, &value as *const _, 1);
            std::mem::forget(value);
            *self.versions[dst_slice_index].get() = next_component_version();
            self.fragmentation += dst_allocation.estimate_fragmentation() - previous_fragmentation;
        }
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

    fn swap_remove(
        &mut self,
        frame_counter: u64,
        ArchetypeIndex(archetype): ArchetypeIndex,
        index: usize,
    ) {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        let previous_fragmentation = allocation.estimate_fragmentation();
        allocation.swap_remove(frame_counter, index);
        self.slices[slice_index] = allocation.as_raw_slice();
        self.fragmentation += allocation.estimate_fragmentation() - previous_fragmentation;
        self.entity_len -= 1;
    }

    fn pack(&mut self, epoch_threshold: u64) -> usize {
        if size_of::<T>() == 0 {
            return 0;
        }

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

    fn fragmentation(&self) -> f32 { self.fragmentation / self.entity_len as f32 }
}

impl<T: Component> Default for PackedStorage<T> {
    fn default() -> Self {
        Self {
            index: Vec::new(),
            slices: Vec::new(),
            versions: Vec::new(),
            allocations: Vec::new(),
            fragmentation: 0f32,
            entity_len: 0,
        }
    }
}

impl<'a, T: Component> ComponentStorage<'a, T> for PackedStorage<T> {
    type Iter = ComponentIter<'a, T>;
    type IterMut = ComponentIterMut<'a, T>;

    unsafe fn extend_memcopy(
        &mut self,
        epoch: u64,
        ArchetypeIndex(archetype): ArchetypeIndex,
        ptr: *const T,
        count: usize,
    ) {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        let previous_fragmentation = allocation.estimate_fragmentation();
        allocation.extend_memcopy(epoch, ptr, count);
        self.slices[slice_index] = allocation.as_raw_slice();
        self.fragmentation += allocation.estimate_fragmentation() - previous_fragmentation;
        self.entity_len += count;
    }

    fn ensure_capacity(
        &mut self,
        epoch: u64,
        ArchetypeIndex(archetype): ArchetypeIndex,
        capacity: usize,
    ) {
        let slice_index = self.index[archetype as usize];
        let allocation = &mut self.allocations[slice_index];
        allocation.ensure_capacity(epoch, capacity);
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
            storage.extend_memcopy(0, ArchetypeIndex(0), ptr, 3);
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
            storage.extend_memcopy(0, ArchetypeIndex(0), ptr, 3);
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
            storage.extend_memcopy(0, ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);
        }

        storage.swap_remove(0, ArchetypeIndex(0), 0);

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
            storage.extend_memcopy(0, ArchetypeIndex(0), ptr, 3);
            std::mem::forget(components);
        }

        storage.swap_remove(0, ArchetypeIndex(0), 2);

        unsafe {
            let slice = storage.get_mut(ArchetypeIndex(0)).unwrap();
            assert_eq!(slice.into_slice(), &[1usize, 2usize]);
        }
    }
}
