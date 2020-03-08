use crate::borrow::{Exclusive, Shared};
use crate::storage::archetype::Archetype;
use crate::storage::chunk::Chunk;
use crate::storage::components::{Component, ComponentTypeId};
use derivative::Derivative;
use std::any::TypeId;
use std::marker::PhantomData;
use std::slice::{Iter, IterMut};

pub trait View<'data> {
    type Elements: Fetch<'data>;

    unsafe fn fetch(archetype: &'data Archetype, chunk: &'data Chunk) -> Self::Elements;

    fn validate();

    /// Determines if the view reads the specified data type.
    fn reads<T: Component>() -> bool;

    /// Determines if the view writes to the specified data type.
    fn writes<T: Component>() -> bool;
}

pub trait Fetch<'a> {
    type Iter: Iterator + 'a;
    unsafe fn iter_mut(&mut self) -> Self::Iter;
    unsafe fn find_components<T: Component>(&self) -> Option<&[T]>;
    unsafe fn find_components_mut<T: Component>(&mut self) -> Option<&mut [T]>;
}

pub trait ReadOnlyFetch<'a>: Fetch<'a> + ReadOnly {
    unsafe fn iter(&self) -> Self::Iter;
}

#[doc(hidden)]
pub trait ReadOnly {}

/// Reads a single entity data component type from a chunk.
#[derive(Derivative, Debug)]
#[derivative(Default(bound = ""))]
pub struct Read<T: Component>(PhantomData<T>);

pub struct ReadFetch<'a, T: Component> {
    ptr: *const T,
    len: usize,
    _borrow: Shared<'a>,
}

impl<T: Component> ReadOnly for Read<T> {}
impl<T: Component> Copy for Read<T> {}
impl<T: Component> Clone for Read<T> {
    fn clone(&self) -> Self { *self }
}

impl<'world, T: Component> View<'world> for Read<T> {
    type Elements = ReadFetch<'world, T>;

    fn validate() {}

    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    fn writes<D: Component>() -> bool { false }

    unsafe fn fetch(_: &'world Archetype, chunk: &'world Chunk) -> Self::Elements {
        let type_id = ComponentTypeId::of::<T>();
        let (borrow, slice) = chunk
            .components()
            .get(type_id)
            .unwrap()
            .get_slice()
            .deconstruct();
        ReadFetch {
            ptr: slice.as_ptr(),
            len: slice.len(),
            _borrow: borrow,
        }
    }
}

impl<'a, T: Component> Fetch<'a> for ReadFetch<'a, T> {
    type Iter = Iter<'a, T>;

    unsafe fn iter_mut(&mut self) -> Self::Iter {
        std::slice::from_raw_parts(self.ptr, self.len).iter()
    }

    unsafe fn find_components<D: Component>(&self) -> Option<&[D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            Some(std::slice::from_raw_parts(self.ptr as *const D, self.len))
        } else {
            None
        }
    }

    unsafe fn find_components_mut<D: Component>(&mut self) -> Option<&mut [D]> { None }
}

impl<'a, T: Component> ReadOnly for ReadFetch<'a, T> {}

impl<'a, T: Component> ReadOnlyFetch<'a> for ReadFetch<'a, T> {
    unsafe fn iter(&self) -> Self::Iter { std::slice::from_raw_parts(self.ptr, self.len).iter() }
}

/// Reads a single entity data component type from a chunk.
#[derive(Derivative, Debug)]
#[derivative(Default(bound = ""))]
pub struct TryRead<T: Component>(PhantomData<T>);

pub struct TryReadFetch<'a, T: Component> {
    slice: Option<(Shared<'a>, *const T, usize)>,
    len: usize,
}

impl<T: Component> ReadOnly for TryRead<T> {}
impl<T: Component> Copy for TryRead<T> {}
impl<T: Component> Clone for TryRead<T> {
    fn clone(&self) -> Self { *self }
}

impl<'world, T: Component> View<'world> for TryRead<T> {
    type Elements = TryReadFetch<'world, T>;

    fn validate() {}

    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    fn writes<D: Component>() -> bool { false }

    unsafe fn fetch(_: &'world Archetype, chunk: &'world Chunk) -> Self::Elements {
        let type_id = ComponentTypeId::of::<T>();
        TryReadFetch {
            slice: chunk.components().get(type_id).map(|s| {
                let (borrow, slice) = s.get_slice::<T>().deconstruct();
                (borrow, slice.as_ptr(), slice.len())
            }),
            len: chunk.len(),
        }
    }
}

impl<'a, T: Component> Fetch<'a> for TryReadFetch<'a, T> {
    type Iter = TryReadIter<'a, T>;

    unsafe fn iter_mut(&mut self) -> Self::Iter { self.iter() }

    unsafe fn find_components<D: Component>(&self) -> Option<&[D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            if let Some((_, ptr, len)) = &self.slice {
                return Some(std::slice::from_raw_parts(*ptr as *const D, *len));
            }
        }
        None
    }

    unsafe fn find_components_mut<D: Component>(&mut self) -> Option<&mut [D]> { None }
}

impl<'a, T: Component> ReadOnly for TryReadFetch<'a, T> {}

impl<'a, T: Component> ReadOnlyFetch<'a> for TryReadFetch<'a, T> {
    unsafe fn iter(&self) -> Self::Iter {
        if let Some((_, ptr, len)) = &self.slice {
            TryReadIter::Found(std::slice::from_raw_parts(*ptr, *len).iter())
        } else {
            TryReadIter::Missing(self.len)
        }
    }
}

pub enum TryReadIter<'a, T> {
    Found(Iter<'a, T>),
    Missing(usize),
}

impl<'a, T> Iterator for TryReadIter<'a, T> {
    type Item = Option<&'a T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Found(iter) => iter.next().map(Some),
            Self::Missing(remaining) => {
                if *remaining > 0 {
                    *remaining -= 1;
                    Some(None)
                } else {
                    None
                }
            }
        }
    }
}

/// Writes a single entity data component type from a chunk.
#[derive(Derivative, Debug)]
#[derivative(Default(bound = ""))]
pub struct Write<T: Component>(PhantomData<T>);

pub struct WriteFetch<'a, T: Component> {
    ptr: *mut T,
    len: usize,
    _borrow: Exclusive<'a>,
}

impl<T: Component> ReadOnly for Write<T> {}
impl<T: Component> Copy for Write<T> {}
impl<T: Component> Clone for Write<T> {
    fn clone(&self) -> Self { *self }
}

impl<'world, T: Component> View<'world> for Write<T> {
    type Elements = WriteFetch<'world, T>;

    fn validate() {}

    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    fn writes<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    unsafe fn fetch(_: &'world Archetype, chunk: &'world Chunk) -> Self::Elements {
        let type_id = ComponentTypeId::of::<T>();
        let (borrow, slice) = chunk
            .components()
            .get(type_id)
            .unwrap()
            .get_slice_mut()
            .deconstruct();
        WriteFetch {
            ptr: slice.as_mut_ptr(),
            len: slice.len(),
            _borrow: borrow,
        }
    }
}

impl<'a, T: Component> Fetch<'a> for WriteFetch<'a, T> {
    type Iter = IterMut<'a, T>;

    unsafe fn iter_mut(&mut self) -> Self::Iter {
        std::slice::from_raw_parts_mut(self.ptr, self.len).iter_mut()
    }

    unsafe fn find_components<D: Component>(&self) -> Option<&[D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            Some(std::slice::from_raw_parts(self.ptr as *const D, self.len))
        } else {
            None
        }
    }

    unsafe fn find_components_mut<D: Component>(&mut self) -> Option<&mut [D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            Some(std::slice::from_raw_parts_mut(self.ptr as *mut D, self.len))
        } else {
            None
        }
    }
}

/// Reads a single entity data component type from a chunk.
#[derive(Derivative, Debug)]
#[derivative(Default(bound = ""))]
pub struct TryWrite<T: Component>(PhantomData<T>);

pub struct TryWriteFetch<'a, T: Component> {
    slice: Option<(Exclusive<'a>, *mut T, usize)>,
    len: usize,
}

impl<T: Component> ReadOnly for TryWrite<T> {}
impl<T: Component> Copy for TryWrite<T> {}
impl<T: Component> Clone for TryWrite<T> {
    fn clone(&self) -> Self { *self }
}

impl<'world, T: Component> View<'world> for TryWrite<T> {
    type Elements = TryWriteFetch<'world, T>;

    fn validate() {}

    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    fn writes<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    unsafe fn fetch(_: &'world Archetype, chunk: &'world Chunk) -> Self::Elements {
        let type_id = ComponentTypeId::of::<T>();
        TryWriteFetch {
            slice: chunk.components().get(type_id).map(|s| {
                let (borrow, slice) = s.get_slice_mut::<T>().deconstruct();
                (borrow, slice.as_mut_ptr(), slice.len())
            }),
            len: chunk.len(),
        }
    }
}

impl<'a, T: Component> Fetch<'a> for TryWriteFetch<'a, T> {
    type Iter = TryWriteIter<'a, T>;

    unsafe fn iter_mut(&mut self) -> Self::Iter {
        if let Some((_, ptr, len)) = &self.slice {
            TryWriteIter::Found(std::slice::from_raw_parts_mut(*ptr, *len).iter_mut())
        } else {
            TryWriteIter::Missing(self.len)
        }
    }

    unsafe fn find_components<D: Component>(&self) -> Option<&[D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            if let Some((_, ptr, len)) = &self.slice {
                return Some(std::slice::from_raw_parts(*ptr as *const D, *len));
            }
        }

        None
    }

    unsafe fn find_components_mut<D: Component>(&mut self) -> Option<&mut [D]> {
        if TypeId::of::<D>() == TypeId::of::<T>() {
            if let Some((_, ptr, len)) = &self.slice {
                return Some(std::slice::from_raw_parts_mut(*ptr as *mut D, *len));
            }
        }

        None
    }
}

pub enum TryWriteIter<'a, T> {
    Found(IterMut<'a, T>),
    Missing(usize),
}

impl<'a, T> Iterator for TryWriteIter<'a, T> {
    type Item = Option<&'a mut T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Found(iter) => iter.next().map(Some),
            Self::Missing(remaining) => {
                if *remaining > 0 {
                    *remaining -= 1;
                    Some(None)
                } else {
                    None
                }
            }
        }
    }
}
