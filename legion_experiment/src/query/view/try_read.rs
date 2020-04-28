use super::{DefaultFilter, Fetch, IntoIndexableIter, ReadOnlyFetch, View};
use crate::{
    iter::indexed::{IndexedIter, TrustedRandomAccess},
    query::{
        filter::{passthrough::Passthrough, EntityFilterTuple},
        QueryResult,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::{Component, ComponentTypeId},
        ComponentSlice, ComponentStorage, Components,
    },
};
use derivative::Derivative;
use std::{any::TypeId, marker::PhantomData};

/// Reads a single entity data component type from a chunk.
#[derive(Derivative, Debug, Copy, Clone)]
#[derivative(Default(bound = ""))]
pub struct TryRead<T: Component>(PhantomData<T>);

impl<T: Component> DefaultFilter for TryRead<T> {
    type Filter = EntityFilterTuple<Passthrough, Passthrough>;
}

impl<'data, T: Component> View<'data> for TryRead<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = <TryReadIter<'data, T> as Iterator>::Item;
    type Iter = TryReadIter<'data, T>;
    type Read = [ComponentTypeId; 1];
    type Write = [ComponentTypeId; 0];

    #[inline]
    fn validate() {}

    #[inline]
    fn reads_types() -> Self::Read {
        [ComponentTypeId::of::<T>()]
    }

    #[inline]
    fn writes_types() -> Self::Write {
        []
    }

    #[inline]
    fn reads<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    #[inline]
    fn writes<D: Component>() -> bool {
        false
    }

    unsafe fn fetch(
        components: &'data Components,
        archetypes: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        let components = components.get_downcast::<T>().unwrap();
        let archetype_indexes = match query {
            QueryResult::Unordered(archetypes) => archetypes.iter(),
            QueryResult::Ordered(archetypes) => archetypes.iter(),
        };
        TryReadIter {
            components,
            archetypes,
            archetype_indexes,
        }
    }
}

/// A fetch iterator which pulls out shared component slices.
pub struct TryReadIter<'a, T: Component> {
    components: &'a T::Storage,
    archetype_indexes: std::slice::Iter<'a, ArchetypeIndex>,
    archetypes: &'a [Archetype],
}

impl<'a, T: Component> Iterator for TryReadIter<'a, T> {
    type Item = Slice<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.archetype_indexes.next().map(|i| {
            self.components.get(*i).map_or_else(
                || Slice::Empty(self.archetypes[*i].entities().len()),
                |c| c.into(),
            )
        })
    }
}

pub enum Slice<'a, T: Component> {
    Occupied {
        version: &'a u64,
        components: &'a [T],
    },
    Empty(usize),
}

impl<'a, T: Component> From<ComponentSlice<'a, T>> for Slice<'a, T> {
    fn from(slice: ComponentSlice<'a, T>) -> Self {
        Slice::Occupied {
            components: slice.components,
            version: slice.version,
        }
    }
}

impl<'a, T: Component> IntoIndexableIter for Slice<'a, T> {
    type Item = Option<&'a T>;
    type IntoIter = IndexedIter<Data<'a, T>>;

    fn into_indexable_iter(self) -> Self::IntoIter {
        let data = match self {
            Self::Occupied { components, .. } => Data::Occupied(components),
            Self::Empty(count) => Data::Empty(count),
        };
        IndexedIter::new(data)
    }
}

impl<'a, T: Component> IntoIterator for Slice<'a, T> {
    type Item = <Self as IntoIndexableIter>::Item;
    type IntoIter = <Self as IntoIndexableIter>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.into_indexable_iter()
    }
}

unsafe impl<'a, T: Component> ReadOnlyFetch for Slice<'a, T> {
    fn get_components(&self) -> Self::Data {
        match self {
            Self::Occupied { components, .. } => Some(components),
            Self::Empty(_) => None,
        }
    }
}

impl<'a, T: Component> Fetch for Slice<'a, T> {
    type Data = Option<&'a [T]>;

    #[inline]
    fn into_components(self) -> Self::Data {
        match self {
            Self::Occupied { components, .. } => Some(components),
            Self::Empty(_) => None,
        }
    }

    #[inline]
    fn find<C: 'static>(&self) -> Option<&[C]> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            // safety: C and T are the same type
            match self {
                Self::Occupied { components, .. } => Some(unsafe {
                    std::slice::from_raw_parts(components.as_ptr() as *const C, components.len())
                }),
                Self::Empty(_) => None,
            }
        } else {
            None
        }
    }

    #[inline]
    fn find_mut<C: 'static>(&mut self) -> Option<&mut [C]> {
        None
    }

    #[inline]
    fn version<C: Component>(&self) -> Option<u64> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            match self {
                Self::Occupied { version, .. } => Some(**version),
                Self::Empty(_) => None,
            }
        } else {
            None
        }
    }

    #[inline]
    fn accepted(&mut self) {}
}

pub enum Data<'a, T: Component> {
    Occupied(&'a [T]),
    Empty(usize),
}

unsafe impl<'a, T: Component> TrustedRandomAccess for Data<'a, T> {
    type Item = Option<&'a T>;

    #[inline]
    fn len(&self) -> usize {
        match self {
            Self::Occupied(slice) => slice.len(),
            Self::Empty(len) => *len,
        }
    }

    #[inline]
    unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item {
        match self {
            Self::Occupied(slice) => Some(slice.get_unchecked(i)),
            Self::Empty(_) => None,
        }
    }

    #[inline]
    fn split_at(self, index: usize) -> (Self, Self) {
        match self {
            Self::Occupied(slice) => {
                let (left, right) = slice.split_at(index);
                (Self::Occupied(left), Self::Occupied(right))
            }
            Self::Empty(count) => (Self::Empty(index), Self::Empty(count - index)),
        }
    }
}
