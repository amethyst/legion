use crate::borrow::RefMapMut;
use crate::entity::Entity;
use crate::query::view::{Fetch, ReadOnly, View};
use crate::storage::archetype::Archetype;
use crate::storage::chunk::Chunk;
use crate::storage::components::Component;
use crate::storage::filter::{EntityFilter, EntityFilterTuple, FilterResult};
use crate::storage::index::ArchetypeIndex;
use crate::storage::tags::Tag;
use crate::storage::{
    ArchetypeFilter, ArchetypeSearchIter, ChunkFilter, LayoutFilter, LayoutIndex,
};
use crate::world::WorldId;
use std::collections::HashMap;
use std::iter::{Enumerate, Fuse, Iterator};
use std::marker::PhantomData;
use std::ops::Index;
use std::slice::Iter;
use view::ReadOnlyFetch;

mod filter;
mod view;

pub struct ChunkView<'a, V: for<'b> View<'b>> {
    archetype: &'a Archetype,
    chunk: &'a Chunk,
    fetch: <V as View<'a>>::Elements,
}

impl<'a, V: for<'b> View<'b>> ChunkView<'a, V> {
    unsafe fn new(archetype: &'a Archetype, chunk: &'a Chunk) -> Self {
        Self {
            archetype,
            chunk,
            fetch: V::fetch(archetype, chunk),
        }
    }

    #[inline]
    pub fn entities(&self) -> &'a [Entity] { &self.chunk }

    #[inline]
    pub fn iter(&self) -> <<V as View<'a>>::Elements as Fetch<'a>>::Iter
    where
        <V as View<'a>>::Elements: ReadOnlyFetch<'a>,
    {
        unsafe { self.fetch.iter() }
    }

    #[inline]
    pub fn iter_mut(&mut self) -> <<V as View<'a>>::Elements as Fetch<'a>>::Iter {
        unsafe { self.fetch.iter_mut() }
    }

    #[inline]
    pub fn iter_entities(
        &self,
    ) -> ZipEntities<
        'a,
        <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
        <<V as View<'a>>::Elements as Fetch<'a>>::Iter,
    >
    where
        <V as View<'a>>::Elements: ReadOnlyFetch<'a>,
    {
        ZipEntities {
            entities: &self.chunk,
            data: unsafe { self.fetch.iter() },
            index: 0,
            _item: PhantomData,
        }
    }

    #[inline]
    pub fn iter_entities_mut(
        &mut self,
    ) -> ZipEntities<
        'a,
        <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
        <<V as View<'a>>::Elements as Fetch<'a>>::Iter,
    > {
        ZipEntities {
            entities: &self.chunk,
            data: unsafe { self.fetch.iter_mut() },
            index: 0,
            _item: PhantomData,
        }
    }

    pub fn tag<T: Tag>(&self) -> Option<&T> { self.archetype.tags().get() }

    pub fn components<T: Component>(&self) -> Option<&[T]> {
        if !V::reads::<T>() {
            panic!("component type not readable via this query");
        }
        unsafe { self.fetch.find_components() }
    }

    pub fn components_mut<T: Component>(&mut self) -> Option<&mut [T]> {
        if !V::writes::<T>() {
            panic!("component type not writable via this query");
        }
        unsafe { self.fetch.find_components_mut() }
    }
}

/// An iterator which yields view data tuples and entity IDs from a `Chunk`.
pub struct ZipEntities<'data, T, I: Iterator<Item = T>> {
    entities: &'data [Entity],
    data: I,
    index: usize,
    _item: PhantomData<T>,
}

impl<'data, T, I: Iterator<Item = T>> Iterator for ZipEntities<'data, T, I> {
    type Item = (Entity, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(data) = self.data.next() {
            let i = self.index;
            self.index += 1;
            unsafe { Some((*self.entities.get_unchecked(i), data)) }
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.entities.len() - self.index;
        (len, Some(len))
    }
}

impl<'data, T, I: Iterator<Item = T>> ExactSizeIterator for ZipEntities<'data, T, I> {}

pub trait EntitySource: Index<ArchetypeIndex, Output = Archetype> {
    fn id(&self) -> WorldId;
    fn layout_index(&self) -> &LayoutIndex;
    fn archetypes(&self) -> &[Archetype];
}

pub trait QueryFilter<'a>: EntityFilter {
    type Matches: Iterator<Item = (&'a Archetype, &'a Chunk)> + 'a;

    fn iter_chunks<T: EntitySource>(&'a mut self, world: &'a T) -> Self::Matches;
}

impl<'a, L, A, C> QueryFilter<'a> for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter + Send + Sync + Clone + 'a,
    A: ArchetypeFilter + Send + Sync + Clone + 'a,
    C: ChunkFilter + Send + Sync + Clone + 'a,
{
    type Matches = ChunkIter<'a, ArchetypeSearchIter<'a, L, A>, C>;

    fn iter_chunks<T: EntitySource>(&'a mut self, world: &'a T) -> Self::Matches {
        let (layout_filter, arch_filter, chunk_filter) = self.filters();
        let archetype_indexes = world.layout_index().search(layout_filter, arch_filter);
        ChunkIter {
            archetype_indexes,
            archetypes: world.archetypes(),
            current_archetype: None,
            filter: chunk_filter,
        }
    }
}

pub struct ChunkIter<'a, I: Iterator<Item = ArchetypeIndex>, F: ChunkFilter> {
    archetype_indexes: I,
    archetypes: &'a [Archetype],
    current_archetype: Option<(&'a Archetype, Iter<'a, Chunk>)>,
    filter: &'a mut F,
}

impl<'a, I: Iterator<Item = ArchetypeIndex>, F: ChunkFilter> Iterator for ChunkIter<'a, I, F> {
    type Item = (&'a Archetype, &'a Chunk);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let filter = &mut self.filter;
            if let Some((arch, chunks)) = &mut self.current_archetype {
                if let Some(chunk) = chunks.find(|chunk| filter.matches_chunk(chunk).is_pass()) {
                    return Some((arch, chunk));
                }
            }

            if let Some(index) = self.archetype_indexes.next() {
                let arch = &self.archetypes[index];
                self.current_archetype = Some((arch, arch.iter()));
            } else {
                return None;
            }
        }
    }
}

#[derive(Clone)]
pub struct CachingEntityFilter<F: EntityFilter> {
    archetypes: HashMap<WorldId, (usize, Vec<ArchetypeIndex>)>,
    filter: F,
}

impl<F: EntityFilter> CachingEntityFilter<F> {
    fn prepare(&mut self, archetypes: &[Archetype], world: WorldId) -> &[ArchetypeIndex] {
        let (cursor, matched_archetypes) = self
            .archetypes
            .entry(world)
            .or_insert_with(|| (0, Vec::new()));

        let (layout_filter, arch_filter) = self.filter.static_filters();
        for (i, arch) in archetypes.iter().skip(*cursor).enumerate() {
            let arch_index = ArchetypeIndex(*cursor + i);
            if layout_filter
                .matches_layout(arch.layout().component_types(), arch.layout().tag_types())
                .is_pass()
                && arch_filter.matches_archetype(&arch.tags()).is_pass()
            {
                matched_archetypes.push(arch_index);
            }
        }

        let added = &matched_archetypes[*cursor..];
        *cursor = archetypes.len();
        added
    }
}

impl<F: EntityFilter> EntityFilter for CachingEntityFilter<F> {
    type Layout = F::Layout;
    type Archetype = F::Archetype;
    type Chunk = F::Chunk;

    fn static_filters(&self) -> (&Self::Layout, &Self::Archetype) { self.filter.static_filters() }

    fn filters(&mut self) -> (&Self::Layout, &Self::Archetype, &mut Self::Chunk) {
        self.filter.filters()
    }

    fn into_filters(self) -> (Self::Layout, Self::Archetype, Self::Chunk) {
        self.filter.into_filters()
    }
}

static EMPTY_CACHE: [ArchetypeIndex; 0] = [];

impl<'a, F: EntityFilter + 'a> QueryFilter<'a> for CachingEntityFilter<F> {
    type Matches = ChunkIter<
        'a,
        CachedFilterIter<'a, <F as EntityFilter>::Layout, <F as EntityFilter>::Archetype>,
        <F as EntityFilter>::Chunk,
    >;

    fn iter_chunks<T: EntitySource>(&'a mut self, world: &'a T) -> Self::Matches {
        let world_id = world.id();
        let (start, cache) = self
            .archetypes
            .get(&world_id)
            .map(|(start, cache)| (*start, cache.as_slice()))
            .unwrap_or_else(|| (0, &EMPTY_CACHE));
        let (layout_filter, arch_filter, chunk_filter) = self.filter.filters();
        let archetype_indexes = CachedFilterIter {
            cache: cache.iter().fuse(),
            layout_filter,
            arch_filter,
            start,
            archetypes: world.archetypes()[start..].iter().enumerate(),
        };
        ChunkIter {
            archetype_indexes,
            archetypes: world.archetypes(),
            current_archetype: None,
            filter: chunk_filter,
        }
    }
}

pub struct CachedFilterIter<'a, L: LayoutFilter, A: ArchetypeFilter> {
    cache: Fuse<Iter<'a, ArchetypeIndex>>,
    layout_filter: &'a L,
    arch_filter: &'a A,
    start: usize,
    archetypes: Enumerate<Iter<'a, Archetype>>,
}

impl<'a, L: LayoutFilter, A: ArchetypeFilter> Iterator for CachedFilterIter<'a, L, A> {
    type Item = ArchetypeIndex;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(index) = self.cache.next() {
            return Some(*index);
        }

        for (i, arch) in &mut self.archetypes {
            let arch_index = ArchetypeIndex(self.start + i);
            if self
                .layout_filter
                .matches_layout(arch.layout().component_types(), arch.layout().tag_types())
                .is_pass()
                && self.arch_filter.matches_archetype(&arch.tags()).is_pass()
            {
                return Some(arch_index);
            }
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let cache_len = self.cache.size_hint().0;
        let arch_len = self.archetypes.size_hint().1;
        (cache_len, arch_len.map(|len| cache_len + len - self.start))
    }
}

pub struct Query<V: for<'a> View<'a>, F: for<'a> QueryFilter<'a>, S: EntitySource> {
    _view: PhantomData<V>,
    _source: PhantomData<S>,
    filter: F,
}

impl<V: for<'a> View<'a>, F: for<'a> QueryFilter<'a>, S: EntitySource> Query<V, F, S> {
    /// Adds an additional filter to the query.
    pub fn filter<T: for<'a> QueryFilter<'a>>(
        self,
        filter: T,
    ) -> Query<V, <F as std::ops::BitAnd<T>>::Output, S>
    where
        F: std::ops::BitAnd<T>,
        <F as std::ops::BitAnd<T>>::Output: for<'a> QueryFilter<'a>,
    {
        Query {
            _view: self._view,
            _source: self._source,
            filter: self.filter & filter,
        }
    }

    pub unsafe fn iter_chunks_unchecked<'a>(
        &'a mut self,
        world: &'a S,
    ) -> ChunkViewIter<'a, <F as QueryFilter<'a>>::Matches, V> {
        let chunks = self.filter.iter_chunks(world);
        ChunkViewIter {
            _view: PhantomData,
            chunks,
        }
    }

    pub fn iter_chunks<'a>(
        &'a mut self,
        world: &'a S,
    ) -> ChunkViewIter<'a, <F as QueryFilter<'a>>::Matches, V>
    where
        V: ReadOnly,
    {
        // safe because the view can only read data immutably
        unsafe { self.iter_chunks_unchecked(world) }
    }

    pub fn iter_chunks_mut<'a>(
        &'a mut self,
        world: &'a mut S,
    ) -> ChunkViewIter<'a, <F as QueryFilter<'a>>::Matches, V> {
        // safe because the &mut World ensures exclusivity
        unsafe { self.iter_chunks_unchecked(world) }
    }

    pub unsafe fn iter_unchecked<'a>(
        &'a mut self,
        world: &'a S,
    ) -> impl Iterator<Item = <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item>
    {
        self.iter_chunks_unchecked(world)
            .flat_map(|mut chunk| chunk.iter_mut())
    }

    pub fn iter<'a>(
        &'a mut self,
        world: &'a S,
    ) -> impl Iterator<Item = <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item>
    where
        V: ReadOnly,
    {
        // safe because the view can only read data immutably
        unsafe { self.iter_unchecked(world) }
    }

    pub fn iter_mut<'a>(
        &'a mut self,
        world: &'a mut S,
    ) -> impl Iterator<Item = <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item>
    {
        // safe because the &mut World ensures exclusivity
        unsafe { self.iter_unchecked(world) }
    }

    pub unsafe fn iter_entities_unchecked<'a>(
        &'a mut self,
        world: &'a S,
    ) -> impl Iterator<
        Item = (
            Entity,
            <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
        ),
    > {
        self.iter_chunks_unchecked(world)
            .flat_map(|mut chunk| chunk.iter_entities_mut())
    }

    pub fn iter_entities<'a>(
        &'a mut self,
        world: &'a S,
    ) -> impl Iterator<
        Item = (
            Entity,
            <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
        ),
    >
    where
        V: ReadOnly,
    {
        // safe because the view can only read data immutably
        unsafe { self.iter_entities_unchecked(world) }
    }

    pub fn iter_entities_mut<'a>(
        &'a mut self,
        world: &'a mut S,
    ) -> impl Iterator<
        Item = (
            Entity,
            <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
        ),
    > {
        // safe because the &mut World ensures exclusivity
        unsafe { self.iter_entities_unchecked(world) }
    }
}

pub struct ChunkViewIter<'a, I: Iterator<Item = (&'a Archetype, &'a Chunk)>, V: for<'b> View<'b>> {
    _view: PhantomData<V>,
    chunks: I,
}

impl<'a, I: Iterator<Item = (&'a Archetype, &'a Chunk)>, V: for<'b> View<'b>> Iterator
    for ChunkViewIter<'a, I, V>
{
    type Item = ChunkView<'a, V>;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks
            .next()
            .map(|(archetype, chunk)| unsafe { ChunkView::new(archetype, chunk) })
    }
}

pub struct ChunkEntityIter<'a, I: Iterator<Item = (&'a Archetype, &'a Chunk)>, V: for<'b> View<'b>>
{
    _view: PhantomData<V>,
    chunks: ChunkViewIter<'a, I, V>,
    current: Option<
        ZipEntities<
            'a,
            <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
            <<V as View<'a>>::Elements as Fetch<'a>>::Iter,
        >,
    >,
}

impl<'a, I: Iterator<Item = (&'a Archetype, &'a Chunk)>, V: for<'b> View<'b>> Iterator
    for ChunkEntityIter<'a, I, V>
{
    type Item = (
        Entity,
        <<<V as View<'a>>::Elements as Fetch<'a>>::Iter as Iterator>::Item,
    );

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(iter) = &mut self.current {
                if let Some(item) = iter.next() {
                    return Some(item);
                }
            }

            match self.chunks.next() {
                Some(mut view) => {
                    self.current = Some(view.iter_entities_mut());
                }
                None => return None,
            };
        }
    }
}
