#![doc(hidden)]

use std::{any::TypeId, marker::PhantomData};

use super::{DefaultFilter, Fetch, IntoIndexableIter, IntoView, ReadOnly, ReadOnlyFetch, View};
use crate::internals::{
    iter::indexed::{IndexedIter, TrustedRandomAccess},
    permissions::Permissions,
    query::{
        filter::{passthrough::Passthrough, try_component::TryComponentFilter, EntityFilterTuple},
        QueryResult,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::{Component, ComponentTypeId},
        ComponentSlice, ComponentStorage, Components,
    },
    subworld::ComponentAccess,
};

/// Reads a single entity data component type from a chunk.
#[derive(Debug, Copy, Clone)]
pub struct TryRead<T>(PhantomData<*const T>);

impl<T> Default for TryRead<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

unsafe impl<T> Send for TryRead<T> {}
unsafe impl<T: Sync> Sync for TryRead<T> {}
unsafe impl<T> ReadOnly for TryRead<T> {}

impl<T: Component> DefaultFilter for TryRead<T> {
    type Filter = EntityFilterTuple<TryComponentFilter<T>, Passthrough>;
}

impl<T: Component> IntoView for TryRead<T> {
    type View = Self;
}

impl<'data, T: Component> View<'data> for TryRead<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = Slice<'data, T>;
    type Iter = TryReadIter<'data, T>;
    type Read = [ComponentTypeId; 1];
    type Write = [ComponentTypeId; 0];

    #[inline]
    fn validate() {}

    #[inline]
    fn validate_access(access: &ComponentAccess) -> bool {
        access.allows_read(ComponentTypeId::of::<T>())
    }

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

    #[inline]
    fn requires_permissions() -> Permissions<ComponentTypeId> {
        let mut permissions = Permissions::default();
        permissions.push_read(ComponentTypeId::of::<T>());
        permissions
    }

    unsafe fn fetch(
        components: &'data Components,
        archetypes: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        let components = components.get_downcast::<T>();
        let archetype_indexes = query.index().iter();
        TryReadIter {
            components,
            archetypes,
            archetype_indexes,
        }
    }
}

#[doc(hidden)]
pub struct TryReadIter<'a, T: Component> {
    components: Option<&'a T::Storage>,
    archetype_indexes: std::slice::Iter<'a, ArchetypeIndex>,
    archetypes: &'a [Archetype],
}

impl<'a, T: Component> Iterator for TryReadIter<'a, T> {
    type Item = Option<Slice<'a, T>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.archetype_indexes.next().map(|i| {
            let slice = self
                .components
                .and_then(|components| components.get(*i))
                .map_or_else(
                    || Slice::Empty(self.archetypes[*i].entities().len()),
                    |c| c.into(),
                );
            Some(slice)
        })
    }
}

#[doc(hidden)]
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
                Self::Occupied { components, .. } => {
                    Some(unsafe {
                        std::slice::from_raw_parts(
                            components.as_ptr() as *const C,
                            components.len(),
                        )
                    })
                }
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

#[doc(hidden)]
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
            Self::Empty(count) => {
                debug_assert!(index < count);
                (Self::Empty(index), Self::Empty(count - index))
            }
        }
    }
}
