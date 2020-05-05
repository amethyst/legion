use crate::{
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::Component,
        group::SubGroup,
    },
    world::{World, WorldId},
};
use filter::{passthrough::Passthrough, DynamicFilter, EntityFilter, GroupMatcher};
use std::{
    collections::HashMap,
    iter::{Copied, Zip},
    marker::PhantomData,
    slice::Iter,
};
use view::{Fetch, IntoIndexableIter, ReadOnlyFetch, View};

pub mod filter;
pub mod view;

pub trait IntoQuery: for<'a> View<'a> {
    fn query() -> Query<Self, Self::Filter>;
}

impl<T: for<'a> View<'a>> IntoQuery for T {
    fn query() -> Query<Self, Self::Filter> {
        Self::validate();

        Query {
            _view: PhantomData,
            filter: <Self::Filter as Default>::default(),
            layout_matches: HashMap::new(),
        }
    }
}

/// Contains the result of an entity layout filter.
#[derive(Debug, Clone)]
pub enum QueryResult<'a> {
    /// Archetypes must be fetched out-of-order.
    Unordered(&'a [ArchetypeIndex]),
    /// Archetypes can be assumed to be stored in components storage in the same
    /// order as specified in this result.
    Ordered(&'a [ArchetypeIndex]),
}

impl<'a> QueryResult<'a> {
    pub fn is_empty(&self) -> bool {
        let archetypes = match self {
            Self::Unordered(archetypes) => archetypes,
            Self::Ordered(archetypes) => archetypes,
        };
        archetypes.is_empty()
    }
}

enum Mode {
    Unordered {
        archetypes: Vec<ArchetypeIndex>,
        seen: usize,
    },
    Ordered {
        group: usize,
        subgroup: SubGroup,
    },
}

pub struct Query<V: for<'a> View<'a>, F: EntityFilter> {
    _view: PhantomData<V>,
    filter: F,
    layout_matches: HashMap<WorldId, Mode>,
}

impl<V: for<'a> View<'a>, F: EntityFilter> Query<V, F> {
    /// Adds an additional filter to the query.
    pub fn filter<T: EntityFilter>(self, filter: T) -> Query<V, <F as std::ops::BitAnd<T>>::Output>
    where
        F: std::ops::BitAnd<T>,
        <F as std::ops::BitAnd<T>>::Output: EntityFilter,
    {
        Query {
            _view: self._view,
            filter: self.filter & filter,
            layout_matches: HashMap::default(),
        }
    }

    #[inline]
    pub unsafe fn iter_unchecked<'a>(
        &'a mut self,
        world: &'a World,
    ) -> std::iter::Flatten<
        ChunkIter<
            'a,
            'a,
            Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
            F,
            <V as View<'a>>::Fetch,
        >,
    > {
        self.iter_chunks_unchecked(world).flatten()
    }

    #[inline]
    pub fn iter_mut<'a>(
        &'a mut self,
        world: &'a mut World,
    ) -> std::iter::Flatten<
        ChunkIter<
            'a,
            'a,
            Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
            F,
            <V as View<'a>>::Fetch,
        >,
    > {
        // safety: we have exclusive access to world
        unsafe { self.iter_unchecked(world) }
    }

    #[inline]
    pub fn iter<'a>(
        &'a mut self,
        world: &'a World,
    ) -> std::iter::Flatten<
        ChunkIter<
            'a,
            'a,
            Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
            F,
            <V as View<'a>>::Fetch,
        >,
    >
    where
        <V as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.iter_unchecked(world) }
    }

    pub unsafe fn iter_chunks_unchecked<'a>(
        &'a mut self,
        world: &'a World,
    ) -> ChunkIter<
        'a,
        'a,
        Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
        F,
        <V as View<'a>>::Fetch,
    > {
        let mode = self.layout_matches.entry(world.id()).or_insert_with(|| {
            let mode = if F::can_match_group() {
                let components = F::group_components();
                components
                    .iter()
                    .next()
                    .and_then(|t| world.group(*t))
                    .map(|(i, g)| (i, g.exact_match(&components)))
                    .and_then(|(group, subgroup)| {
                        subgroup.map(|subgroup| Mode::Ordered { group, subgroup })
                    })
            } else {
                None
            };

            mode.unwrap_or_else(|| Mode::Unordered {
                archetypes: Vec::new(),
                seen: 0,
            })
        });

        let (result, indexes) = match mode {
            Mode::Unordered { archetypes, seen } => {
                for archetype in world.layout_index().search_from(&self.filter, *seen) {
                    archetypes.push(archetype);
                }
                *seen = world.archetypes().len() - 1;
                (QueryResult::Unordered(&*archetypes), archetypes.as_slice())
            }
            Mode::Ordered { group, subgroup } => {
                let archetypes = &world.groups()[*group][*subgroup];
                (QueryResult::Ordered(archetypes), archetypes)
            }
        };

        let fetch = <V as View<'a>>::fetch(world.components(), world.archetypes(), result);
        let inner = indexes.iter().copied().zip(fetch);
        ChunkIter {
            inner,
            filter: &mut self.filter,
            archetypes: world.archetypes(),
            max_count: indexes.len(),
        }
    }

    #[inline]
    pub fn iter_chunks_mut<'a>(
        &'a mut self,
        world: &'a mut World,
    ) -> ChunkIter<
        'a,
        'a,
        Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
        F,
        <V as View<'a>>::Fetch,
    > {
        // safety: we have exclusive access to world
        unsafe { self.iter_chunks_unchecked(world) }
    }

    #[inline]
    pub fn iter_chunks<'a>(
        &'a mut self,
        world: &'a World,
    ) -> ChunkIter<
        'a,
        'a,
        Zip<Copied<Iter<'a, ArchetypeIndex>>, <V as View<'a>>::Iter>,
        F,
        <V as View<'a>>::Fetch,
    >
    where
        <V as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.iter_chunks_unchecked(world) }
    }

    #[inline]
    pub unsafe fn for_each_unchecked<'a, Body>(&'a mut self, world: &'a World, mut f: Body)
    where
        Body: FnMut(<V as View<'a>>::Element),
    {
        // we use a nested loop because it is significantly faster than .flatten()
        for chunk in self.iter_chunks_unchecked(world) {
            for entities in chunk {
                f(entities);
            }
        }
    }

    #[inline]
    pub fn for_each_mut<'a, Body>(&'a mut self, world: &'a mut World, f: Body)
    where
        Body: FnMut(<V as View<'a>>::Element),
    {
        // safety: we have exclusive access to world
        unsafe { self.for_each_unchecked(world, f) };
    }

    #[inline]
    pub fn for_each<'a, Body>(&'a mut self, world: &'a World, f: Body)
    where
        Body: FnMut(<V as View<'a>>::Element),
        <V as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.for_each_unchecked(world, f) };
    }
}

pub struct ChunkView<'a, F: Fetch> {
    archetype: &'a Archetype,
    fetch: F,
}

impl<'a, F: Fetch> ChunkView<'a, F> {
    fn new(archetype: &'a Archetype, fetch: F) -> Self { Self { archetype, fetch } }

    pub fn archetype(&self) -> &Archetype { &self.archetype }

    pub fn component_slice<T: Component>(&self) -> Option<&[T]> { self.fetch.find::<T>() }

    pub fn component_slice_mut<T: Component>(&mut self) -> Option<&mut [T]> {
        self.fetch.find_mut::<T>()
    }

    pub fn into_components(self) -> F::Data { self.fetch.into_components() }

    pub fn get_components(&self) -> F::Data
    where
        F: ReadOnlyFetch,
    {
        self.fetch.get_components()
    }
}

impl<'a, F: Fetch> IntoIterator for ChunkView<'a, F> {
    type IntoIter = <F as IntoIndexableIter>::IntoIter;
    type Item = <F as IntoIndexableIter>::Item;
    fn into_iter(self) -> Self::IntoIter { self.fetch.into_indexable_iter() }
}

pub struct ChunkIter<'world, 'query, I, D, F>
where
    I: Iterator<Item = (ArchetypeIndex, F)>,
    F: Fetch,
    D: DynamicFilter + 'query,
{
    inner: I,
    filter: &'query mut D,
    archetypes: &'world [Archetype],
    max_count: usize,
}

impl<'world, 'query, I, D, F> Iterator for ChunkIter<'world, 'query, I, D, F>
where
    I: Iterator<Item = (ArchetypeIndex, F)>,
    F: Fetch,
    D: DynamicFilter + 'query,
{
    type Item = ChunkView<'world, F>;

    fn next(&mut self) -> Option<Self::Item> {
        for (index, mut fetch) in &mut self.inner {
            if self.filter.matches_archetype(&fetch).is_pass() {
                fetch.accepted();
                return Some(ChunkView::new(&self.archetypes[index], fetch));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) { (0, Some(self.max_count)) }
}

impl<'world, 'query, I, F> ExactSizeIterator for ChunkIter<'world, 'query, I, Passthrough, F>
where
    I: Iterator<Item = (ArchetypeIndex, F)>,
    F: Fetch,
{
}

#[cfg(test)]
mod test {
    use super::view::{read::Read, write::Write};
    use super::IntoQuery;
    use crate::world::World;

    #[test]
    fn query() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);

        let mut query = <(Read<usize>, Write<bool>)>::query();
        for (x, y) in query.iter_mut(&mut world) {
            println!("{}, {}", x, y);
        }
        for chunk in query.iter_chunks_mut(&mut world) {
            let (x, y) = chunk.into_components();
            println!("{:?}, {:?}", x, y);
        }
    }
}
