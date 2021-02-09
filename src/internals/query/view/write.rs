#![doc(hidden)]

use std::{any::TypeId, marker::PhantomData, slice::Iter};

use super::{DefaultFilter, Fetch, IntoIndexableIter, IntoView, View};
use crate::internals::{
    iter::indexed::IndexedIter,
    permissions::Permissions,
    query::{
        filter::{component::ComponentFilter, passthrough::Passthrough, EntityFilterTuple},
        QueryResult,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::{Component, ComponentTypeId},
        next_component_version, ComponentSliceMut, ComponentStorage, Components,
    },
    subworld::ComponentAccess,
};

/// Writes a single mutable entity data component type from a chunk.
#[derive(Debug, Copy, Clone)]
pub struct Write<T>(PhantomData<*const T>);

impl<T> Default for Write<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

unsafe impl<T: Send> Send for Write<T> {}
unsafe impl<T> Sync for Write<T> {}

impl<T: Component> DefaultFilter for Write<T> {
    type Filter = EntityFilterTuple<ComponentFilter<T>, Passthrough>;
}

impl<T: Component> IntoView for Write<T> {
    type View = Self;
}

impl<'data, T: Component> View<'data> for Write<T> {
    type Element = <Self::Fetch as IntoIndexableIter>::Item;
    type Fetch = WriteFetch<'data, T>;
    type Iter = WriteIter<'data, T>;
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
        _: &'data [Archetype],
        query: QueryResult<'data>,
    ) -> Self::Iter {
        if query.is_empty() {
            return WriteIter::Empty;
        };

        let components = if let Some(components) = components.get_downcast::<T>() {
            components
        } else {
            return WriteIter::Empty;
        };

        if query.is_ordered() {
            WriteIter::Grouped {
                slices: components.iter_mut(query.range().start, query.range().end),
            }
        } else {
            WriteIter::Indexed {
                components,
                archetypes: query.index().iter(),
            }
        }
    }
}

#[doc(hidden)]
pub enum WriteIter<'a, T: Component> {
    Indexed {
        components: &'a T::Storage,
        archetypes: Iter<'a, ArchetypeIndex>,
    },
    Grouped {
        slices: <T::Storage as ComponentStorage<'a, T>>::IterMut,
    },
    Empty,
}

impl<'a, T: Component> Iterator for WriteIter<'a, T> {
    type Item = Option<WriteFetch<'a, T>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Indexed {
                components,
                archetypes,
            } => {
                archetypes
                    .next()
                    .map(|i| unsafe { components.get_mut(*i).map(|c| c.into()) })
            }
            Self::Grouped { slices } => slices.next().map(|c| Some(c.into())),
            Self::Empty => None,
        }
    }
}

#[doc(hidden)]
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

    fn into_indexable_iter(self) -> Self::IntoIter {
        IndexedIter::new(self.components)
    }
}

impl<'a, T: Component> IntoIterator for WriteFetch<'a, T> {
    type Item = <Self as IntoIndexableIter>::Item;
    type IntoIter = <Self as IntoIndexableIter>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.into_indexable_iter()
    }
}

impl<'a, T: Component> Fetch for WriteFetch<'a, T> {
    type Data = &'a mut [T];

    #[inline]
    fn into_components(self) -> Self::Data {
        self.components
    }

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
    fn accepted(&mut self) {
        *self.version = self.next_version;
    }
}
