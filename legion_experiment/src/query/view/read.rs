use super::{DefaultFilter, Fetch, IntoIndexableIter, ReadOnly, ReadOnlyFetch, View};
use crate::{
    iter::indexed::IndexedIter,
    permissions::Permissions,
    query::{
        filter::{component::ComponentFilter, passthrough::Passthrough, EntityFilterTuple},
        QueryResult,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::{Component, ComponentTypeId},
        ComponentSlice, ComponentStorage, Components,
    },
    subworld::ComponentAccess,
};
use derivative::Derivative;
use std::{any::TypeId, marker::PhantomData, slice::Iter};

/// Reads a single entity data component type from a chunk.
#[derive(Derivative, Debug, Copy, Clone)]
#[derivative(Default(bound = ""))]
pub struct Read<T>(PhantomData<T>);

unsafe impl<T> ReadOnly for Read<T> {}

impl<T: Component> DefaultFilter for Read<T> {
    type Filter = EntityFilterTuple<ComponentFilter<T>, Passthrough>;
}

impl<'data, T: Component> View<'data> for Read<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = <ReadIter<'data, T> as Iterator>::Item;
    type Iter = ReadIter<'data, T>;
    type Read = [ComponentTypeId; 1];
    type Write = [ComponentTypeId; 0];

    #[inline]
    fn validate() {}

    #[inline]
    fn validate_access(access: &ComponentAccess) -> bool {
        access.allows_read(ComponentTypeId::of::<T>())
    }

    #[inline]
    fn reads_types() -> Self::Read { [ComponentTypeId::of::<T>()] }

    #[inline]
    fn writes_types() -> Self::Write { [] }

    #[inline]
    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    #[inline]
    fn writes<D: Component>() -> bool { false }

    #[inline]
    fn requires_permissions() -> Permissions<ComponentTypeId> {
        let mut permissions = Permissions::default();
        permissions.push_read(ComponentTypeId::of::<T>());
        permissions
    }

    unsafe fn fetch(
        components: &'data Components,
        _: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        if query.is_empty() {
            return ReadIter::Empty;
        };
        let components = components.get_downcast::<T>().unwrap();
        if query.is_ordered() {
            ReadIter::Grouped {
                slices: components.iter(query.range().start, query.range().end),
            }
        } else {
            ReadIter::Indexed {
                components,
                archetypes: query.index().iter(),
            }
        }
    }
}

/// A fetch iterator which pulls out shared component slices.
pub enum ReadIter<'a, T: Component> {
    Indexed {
        components: &'a T::Storage,
        archetypes: Iter<'a, ArchetypeIndex>,
    },
    Grouped {
        slices: <T::Storage as ComponentStorage<'a, T>>::Iter,
    },
    Empty,
}

impl<'a, T: Component> Iterator for ReadIter<'a, T> {
    type Item = ReadFetch<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let components = match self {
            Self::Indexed {
                components,
                archetypes,
            } => archetypes.next().and_then(|i| components.get(*i)),
            Self::Grouped { slices } => slices.next(),
            Self::Empty => return None,
        };
        components.map(|c| c.into())
    }
}

pub struct ReadFetch<'a, T: Component> {
    version: &'a u64,
    components: &'a [T],
}

impl<'a, T: Component> From<ComponentSlice<'a, T>> for ReadFetch<'a, T> {
    fn from(slice: ComponentSlice<'a, T>) -> Self {
        ReadFetch {
            components: slice.components,
            version: slice.version,
        }
    }
}

impl<'a, T: Component> IntoIndexableIter for ReadFetch<'a, T> {
    type Item = &'a T;
    type IntoIter = IndexedIter<&'a [T]>;

    fn into_indexable_iter(self) -> Self::IntoIter { IndexedIter::new(self.components) }
}

impl<'a, T: Component> IntoIterator for ReadFetch<'a, T> {
    type Item = <Self as IntoIndexableIter>::Item;
    type IntoIter = <Self as IntoIndexableIter>::IntoIter;

    fn into_iter(self) -> Self::IntoIter { self.into_indexable_iter() }
}

unsafe impl<'a, T: Component> ReadOnlyFetch for ReadFetch<'a, T> {
    #[inline]
    fn get_components(&self) -> Self::Data { self.components }
}

impl<'a, T: Component> Fetch for ReadFetch<'a, T> {
    type Data = &'a [T];

    #[inline]
    fn into_components(self) -> Self::Data { self.components }

    #[inline]
    fn find<C: 'static>(&self) -> Option<&[C]> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            // safety: C and T are the same type
            Some(unsafe {
                std::slice::from_raw_parts(
                    self.components.as_ptr() as *const C,
                    self.components.len(),
                )
            })
        } else {
            None
        }
    }

    #[inline]
    fn find_mut<C: 'static>(&mut self) -> Option<&mut [C]> { None }

    #[inline]
    fn version<C: Component>(&self) -> Option<u64> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            Some(*self.version)
        } else {
            None
        }
    }

    #[inline]
    fn accepted(&mut self) {}
}
