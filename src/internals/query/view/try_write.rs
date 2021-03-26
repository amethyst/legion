#![doc(hidden)]

use std::{any::TypeId, marker::PhantomData};

use super::{DefaultFilter, Fetch, IntoIndexableIter, IntoView, View};
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
        next_component_version, ComponentSliceMut, ComponentStorage, Components,
    },
    subworld::ComponentAccess,
};

/// Writes a single entity data component type from a chunk.
#[derive(Debug, Copy, Clone)]
pub struct TryWrite<T>(PhantomData<*const T>);

impl<T> Default for TryWrite<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

unsafe impl<T: Send> Send for TryWrite<T> {}
unsafe impl<T> Sync for TryWrite<T> {}

impl<T: Component> DefaultFilter for TryWrite<T> {
    type Filter = EntityFilterTuple<TryComponentFilter<T>, Passthrough>;
}

impl<T: Component> IntoView for TryWrite<T> {
    type View = Self;
}

impl<'data, T: Component> View<'data> for TryWrite<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = Slice<'data, T>;
    type Iter = TryWriteIter<'data, T>;
    type Read = [ComponentTypeId; 1];
    type Write = [ComponentTypeId; 1];

    #[inline]
    fn validate() {}

    #[inline]
    fn validate_access(access: &ComponentAccess) -> bool {
        access.allows_write(ComponentTypeId::of::<T>())
    }

    #[inline]
    fn reads_types() -> Self::Read {
        [ComponentTypeId::of::<T>()]
    }

    #[inline]
    fn writes_types() -> Self::Write {
        [ComponentTypeId::of::<T>()]
    }

    #[inline]
    fn reads<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    #[inline]
    fn writes<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    #[inline]
    fn requires_permissions() -> Permissions<ComponentTypeId> {
        let mut permissions = Permissions::default();
        permissions.push(ComponentTypeId::of::<T>());
        permissions
    }

    unsafe fn fetch(
        components: &'data Components,
        archetypes: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        let components = components.get_downcast::<T>();
        let archetype_indexes = query.index().iter();
        TryWriteIter {
            components,
            archetype_indexes,
            archetypes,
        }
    }
}

#[doc(hidden)]
pub struct TryWriteIter<'a, T: Component> {
    components: Option<&'a T::Storage>,
    archetype_indexes: std::slice::Iter<'a, ArchetypeIndex>,
    archetypes: &'a [Archetype],
}

impl<'a, T: Component> Iterator for TryWriteIter<'a, T> {
    type Item = Option<Slice<'a, T>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.archetype_indexes.next().map(|i| unsafe {
            let slice = self
                .components
                .and_then(|components| components.get_mut(*i))
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
        version: &'a mut u64,
        components: &'a mut [T],
        next_version: u64,
    },
    Empty(usize),
}

impl<'a, T: Component> From<ComponentSliceMut<'a, T>> for Slice<'a, T> {
    fn from(slice: ComponentSliceMut<'a, T>) -> Self {
        Slice::Occupied {
            components: slice.components,
            version: slice.version,
            next_version: next_component_version(),
        }
    }
}

impl<'a, T: Component> IntoIndexableIter for Slice<'a, T> {
    type Item = Option<&'a mut T>;
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

impl<'a, T: Component> Fetch for Slice<'a, T> {
    type Data = Option<&'a mut [T]>;

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
        if TypeId::of::<C>() == TypeId::of::<T>() {
            // safety: C and T are the same type
            match self {
                Self::Occupied { components, .. } => {
                    Some(unsafe {
                        std::slice::from_raw_parts_mut(
                            components.as_mut_ptr() as *mut C,
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
    fn accepted(&mut self) {
        if let Self::Occupied {
            version,
            next_version,
            ..
        } = self
        {
            **version = *next_version
        }
    }
}

#[doc(hidden)]
pub enum Data<'a, T: Component> {
    Occupied(&'a mut [T]),
    Empty(usize),
}

unsafe impl<'a, T: Component> TrustedRandomAccess for Data<'a, T> {
    type Item = Option<&'a mut T>;

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
                let (left, right) = slice.split_at_mut(index);
                (Self::Occupied(left), Self::Occupied(right))
            }
            Self::Empty(count) => (Self::Empty(index), Self::Empty(count - index)),
        }
    }
}
