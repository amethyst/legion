use core::any::TypeId;
use std::clone::Clone;
use std::fmt::{Debug, Formatter};
use std::mem::size_of;
use std::ptr::NonNull;

#[cfg(not(feature = "ffi"))]
/// A type ID identifying a tag type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TagTypeId(pub TypeId);

#[cfg(not(feature = "ffi"))]
impl TagTypeId {
    /// Gets the tag type ID that represents type `T`.
    pub fn of<T: Tag>() -> Self { Self(TypeId::of::<T>()) }
}

#[cfg(feature = "ffi")]
/// A type ID identifying a tag type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TagTypeId(pub TypeId, pub u32);

#[cfg(feature = "ffi")]
impl TagTypeId {
    /// Gets the tag type ID that represents type `T`.
    pub fn of<T: Tag>() -> Self { Self(TypeId::of::<T>(), 0) }
}

/// A `Tag` is shared data that can be attached to multiple entities at once.
pub trait Tag: Clone + Send + Sync + PartialEq + 'static {}

impl<T: Clone + Send + Sync + PartialEq + 'static> Tag for T {}

/// Stores metadata describing the tupe of a tag.
#[derive(Copy, Clone, PartialEq)]
pub struct TagMeta {
    size: usize,
    align: usize,
    drop_fn: Option<fn(*mut u8)>,
    eq_fn: fn(*const u8, *const u8) -> bool,
    clone_fn: fn(*const u8, *mut u8),
}

impl TagMeta {
    /// Gets the tag meta of tag type `T`.
    pub fn of<T: Tag>() -> Self {
        TagMeta {
            size: size_of::<T>(),
            align: std::mem::align_of::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(|ptr| unsafe { std::ptr::drop_in_place(ptr as *mut T) })
            } else {
                None
            },
            eq_fn: |a, b| unsafe { *(a as *const T) == *(b as *const T) },
            clone_fn: |src, dst| unsafe {
                let clone = (&*(src as *const T)).clone();
                std::ptr::write(dst as *mut T, clone);
            },
        }
    }

    pub(crate) unsafe fn equals(&self, a: *const u8, b: *const u8) -> bool { (self.eq_fn)(a, b) }

    pub(crate) unsafe fn clone(&self, src: *const u8, dst: *mut u8) { (self.clone_fn)(src, dst) }

    pub(crate) unsafe fn drop(&self, val: *mut u8) {
        if let Some(drop_fn) = self.drop_fn {
            (drop_fn)(val);
        }
    }

    pub(crate) fn layout(&self) -> std::alloc::Layout {
        unsafe { std::alloc::Layout::from_size_align_unchecked(self.size, self.align) }
    }

    pub(crate) fn is_zero_sized(&self) -> bool { self.size == 0 }
}

/// A vector of tag values of a single type.
///
/// Each element in the vector represents the value of tag for
/// the chunk set with the corresponding index within an archetype.
pub struct TagStorage {
    ptr: NonNull<u8>,
    capacity: usize,
    len: usize,
    element: TagMeta,
}

impl TagStorage {
    pub(crate) fn new(element: TagMeta) -> Self {
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

    /// Gets the element metadata.
    pub fn element(&self) -> &TagMeta { &self.element }

    /// Gets the number of tags contained within the vector.
    pub fn len(&self) -> usize { self.len }

    /// Determines if the vector is empty.
    pub fn is_empty(&self) -> bool { self.len() < 1 }

    /// Allocates uninitialized memory for a new element.
    ///
    /// # Safety
    ///
    /// A valid element must be written into the returned address before the
    /// tag storage is next accessed.
    pub unsafe fn alloc_ptr(&mut self) -> *mut u8 {
        if self.len == self.capacity {
            self.grow();
        }

        let ptr = if self.element.size > 0 {
            self.ptr.as_ptr().add(self.len * self.element.size)
        } else {
            self.element.align as *mut u8
        };

        self.len += 1;
        ptr
    }

    /// Pushes a new tag onto the end of the vector.
    ///
    /// # Safety
    ///
    /// Ensure the tag pointed to by `ptr` is representative
    /// of the tag types stored in the vec.
    ///
    /// `ptr` must not point to a location already within the vector.
    ///
    /// The value at `ptr` is _copied_ into the tag vector. If the value
    /// is not `Copy`, then the caller must ensure that the original value
    /// is forgotten with `mem::forget` such that the finalizer is not called
    /// twice and move semantics are maintained.
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

    /// Pushes a new tag onto the end of the vector.
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
        std::mem::forget(value);
    }

    /// Gets a raw pointer to the start of the tag slice.
    ///
    /// Returns a tuple containing `(pointer, element_size, count)`.
    pub fn data_raw(&self) -> (*const u8, usize, usize) {
        (self.ptr.as_ptr(), self.element.size, self.len)
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

    /// Drop the storage without dropping the tags contained in the storage
    pub(crate) fn forget_data(mut self) {
        // this is a bit of a hack, but it makes the Drop impl not drop the elements
        self.element.drop_fn = None;
    }

    fn grow(&mut self) {
        assert_ne!(self.element.size, 0, "capacity overflow");
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
                tracing::error!("out of memory");
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

    #[test]
    fn create_storage() {
        let meta = TagMeta::of::<bool>();
        let storage = TagStorage::new(meta);
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn create_zero_sized() {
        let meta = TagMeta::of::<()>();
        let storage = TagStorage::new(meta);
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn insert() {
        let meta = TagMeta::of::<bool>();
        let mut storage = TagStorage::new(meta);

        let tags = [true, false];
        for tag in &tags {
            unsafe { storage.push(*tag) };
        }

        assert_eq!(storage.len(), 2);
    }

    #[test]
    fn insert_zero_sized() {
        let meta = TagMeta::of::<()>();
        let mut storage = TagStorage::new(meta);

        let tags = [(), ()];
        for tag in &tags {
            unsafe { storage.push(*tag) };
        }

        assert_eq!(storage.len(), 2);
    }

    #[test]
    fn iter() {
        let meta = TagMeta::of::<bool>();
        let mut storage = TagStorage::new(meta);

        let tags = [true, false];
        for tag in &tags {
            unsafe { storage.push(*tag) };
        }

        let slice = unsafe { storage.data_slice::<bool>() };
        assert_eq!(slice.len(), 2);

        for (i, t) in tags.iter().enumerate() {
            assert_eq!(&slice[i], t);
        }
    }

    #[test]
    fn iter_zero_sized() {
        let meta = TagMeta::of::<()>();
        let mut storage = TagStorage::new(meta);

        let tags = [(), ()];
        for tag in &tags {
            unsafe { storage.push(*tag) };
        }

        let slice = unsafe { storage.data_slice::<()>() };
        assert_eq!(slice.len(), 2);

        for (i, t) in tags.iter().enumerate() {
            assert_eq!(&slice[i], t);
        }
    }
}
