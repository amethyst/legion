use super::{DefaultFilter, Fetch, IntoIndexableIter, View};
use crate::{
    iter::indexed::IndexedIter,
    query::{
        filter::{component::ComponentFilter, passthrough::Passthrough, EntityFilterTuple},
        QueryResult,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::{Component, ComponentTypeId},
        packed::next_component_version,
        ComponentSliceMut, ComponentStorage, Components,
    },
};
use derivative::Derivative;
use std::{any::TypeId, marker::PhantomData, slice::Iter};

/// Writes a single mutable entity data component type from a chunk.
#[derive(Derivative, Debug, Copy, Clone)]
#[derivative(Default(bound = ""))]
pub struct Write<T: Component>(PhantomData<T>);

impl<T: Component> DefaultFilter for Write<T> {
    type Filter = EntityFilterTuple<ComponentFilter<T>, Passthrough>;
}

impl<'data, T: Component> View<'data> for Write<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = <WriteIter<'data, T> as Iterator>::Item;
    type Iter = WriteIter<'data, T>;
    type Read = [ComponentTypeId; 1];
    type Write = [ComponentTypeId; 1];

    #[inline]
    fn validate() {}

    #[inline]
    fn reads_types() -> Self::Read { [ComponentTypeId::of::<T>()] }

    #[inline]
    fn writes_types() -> Self::Write { [ComponentTypeId::of::<T>()] }

    #[inline]
    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    #[inline]
    fn writes<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    unsafe fn fetch(
        components: &'data Components,
        _: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        let components = components.get_downcast::<T>().unwrap();
        if query.is_ordered() {
            WriteIter::Grouped {
                slices: components.iter_mut(query.range().start, query.range().end),
            }
        } else {
            WriteIter::Indexed {
                components,
                archetypes: query.into_index().iter(),
            }
        }
    }
}

/// A fetch iterator which pulls out mutable component slices.
pub enum WriteIter<'a, T: Component> {
    Indexed {
        components: &'a T::Storage,
        archetypes: Iter<'a, ArchetypeIndex>,
    },
    Grouped {
        slices: <T::Storage as ComponentStorage<'a, T>>::IterMut,
    },
}

impl<'a, T: Component> Iterator for WriteIter<'a, T> {
    type Item = WriteFetch<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let components = match self {
            Self::Indexed {
                components,
                archetypes,
            } => archetypes
                .next()
                .and_then(|i| unsafe { components.get_mut(*i) }),
            Self::Grouped { slices } => slices.next(),
        };
        components.map(|c| c.into())
    }
}

pub struct WriteFetch<'a, T: Component> {
    version: &'a mut u64,
    components: &'a mut [T],
    next_version: u64,
}

impl<'a, T: Component> From<ComponentSliceMut<'a, T>> for WriteFetch<'a, T> {
    fn from(slice: ComponentSliceMut<'a, T>) -> Self {
        WriteFetch {
            components: slice.components,
            version: slice.version,
            next_version: next_component_version(),
        }
    }
}

impl<'a, T: Component> IntoIndexableIter for WriteFetch<'a, T> {
    type Item = &'a mut T;
    type IntoIter = IndexedIter<&'a mut [T]>;

    fn into_indexable_iter(self) -> Self::IntoIter { IndexedIter::new(self.components) }
}

impl<'a, T: Component> IntoIterator for WriteFetch<'a, T> {
    type Item = <Self as IntoIndexableIter>::Item;
    type IntoIter = <Self as IntoIndexableIter>::IntoIter;

    fn into_iter(self) -> Self::IntoIter { self.into_indexable_iter() }
}

impl<'a, T: Component> Fetch for WriteFetch<'a, T> {
    type Data = &'a mut [T];

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
    fn find_mut<C: 'static>(&mut self) -> Option<&mut [C]> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            // safety: C and T are the same type
            Some(unsafe {
                std::slice::from_raw_parts_mut(
                    self.components.as_mut_ptr() as *mut C,
                    self.components.len(),
                )
            })
        } else {
            None
        }
    }

    #[inline]
    fn version<C: Component>(&self) -> Option<u64> {
        if TypeId::of::<C>() == TypeId::of::<T>() {
            Some(*self.version)
        } else {
            None
        }
    }

    #[inline]
    fn accepted(&mut self) { *self.version = self.next_version; }
}
