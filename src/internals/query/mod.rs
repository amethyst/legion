use crate::internals::{
    entity::Entity,
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::Component,
        group::SubGroup,
    },
    world::{EntityStore, StorageAccessor, WorldId},
};
use filter::{DynamicFilter, EntityFilter, GroupMatcher};
use parking_lot::Mutex;
use std::{collections::HashMap, marker::PhantomData, ops::Range, slice::Iter};
use view::{DefaultFilter, Fetch, IntoIndexableIter, IntoView, ReadOnlyFetch, View};

pub mod filter;
pub mod view;

/// A type (typically a view) which can construct a query.
pub trait IntoQuery: IntoView + Sized {
    /// Constructs a query.
    fn query() -> Query<Self, <Self::View as DefaultFilter>::Filter>;
}

impl<T: IntoView> IntoQuery for T {
    fn query() -> Query<Self, <Self::View as DefaultFilter>::Filter> {
        Self::View::validate();

        Query {
            _view: PhantomData,
            filter: Mutex::new(<<Self::View as DefaultFilter>::Filter as Default>::default()),
            layout_matches: HashMap::new(),
            is_view_filter: true,
        }
    }
}

/// Contains the result of an entity layout filter.
#[derive(Debug, Clone)]
pub struct QueryResult<'a> {
    index: &'a [ArchetypeIndex],
    range: Range<usize>,
    is_ordered: bool,
}

impl<'a> QueryResult<'a> {
    /// The query should access the following archetypes via direct indexing.
    fn unordered(index: &'a [ArchetypeIndex]) -> Self {
        Self {
            range: 0..index.len(),
            index,
            is_ordered: false,
        }
    }

    /// The query should access the following archetypes, and storage can be
    /// assumed to have stored components in the same order as given.
    fn ordered(index: &'a [ArchetypeIndex]) -> Self {
        Self {
            range: 0..index.len(),
            index,
            is_ordered: true,
        }
    }

    /// Gets the archetype index containing the archetypes which match the filter.
    pub fn index(&self) -> &'a [ArchetypeIndex] {
        let (_, slice) = self.index.split_at(self.range.start);
        let (slice, _) = slice.split_at(self.range.len());
        slice
    }

    /// The sub-range of archetypes which should be accessed.
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// Returns `true` if components can be assumed to be stored in the same order as given.
    pub fn is_ordered(&self) -> bool {
        self.is_ordered
    }

    /// The number of archetypes that matches the filter.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Returns `true` if no archetypes matched the filter.
    pub fn is_empty(&self) -> bool {
        self.index().is_empty()
    }

    pub(crate) fn split_at(self, index: usize) -> (Self, Self) {
        (
            Self {
                range: self.range.start..index,
                index: self.index,
                is_ordered: self.is_ordered,
            },
            Self {
                range: index..self.range.end,
                index: self.index,
                is_ordered: self.is_ordered,
            },
        )
    }
}

#[derive(Debug, Clone)]
enum Cache {
    Unordered {
        archetypes: Vec<ArchetypeIndex>,
        seen: usize,
    },
    Ordered {
        group: usize,
        subgroup: SubGroup,
    },
}

/// Provides efficient means to iterate and filter entities in a world.
pub struct Query<V: IntoView, F: EntityFilter = <<V as IntoView>::View as DefaultFilter>::Filter> {
    _view: PhantomData<V>,
    filter: Mutex<F>,
    layout_matches: HashMap<WorldId, Cache>,
    is_view_filter: bool,
}

impl<V: IntoView, F: EntityFilter> Default for Query<V, F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: IntoView, F: EntityFilter> Query<V, F> {
    /// Consructs a new Query
    pub fn new() -> Self {
        V::View::validate();

        Self {
            _view: PhantomData,
            filter: Mutex::new(Default::default()),
            layout_matches: HashMap::new(),
            is_view_filter: true,
        }
    }

    /// Adds an additional filter to the query.
    pub fn filter<T: EntityFilter>(self, filter: T) -> Query<V, <F as std::ops::BitAnd<T>>::Output>
    where
        F: std::ops::BitAnd<T>,
        <F as std::ops::BitAnd<T>>::Output: EntityFilter,
    {
        Query {
            _view: self._view,
            filter: Mutex::new(self.filter.into_inner() & filter),
            layout_matches: HashMap::default(),
            is_view_filter: false,
        }
    }

    // ----------------
    // Query Execution
    // ----------------

    fn validate_archetype_access(storage: &StorageAccessor, archetypes: &[ArchetypeIndex]) {
        for arch in archetypes {
            if !storage.can_access_archetype(*arch) {
                panic!("query attempted to access archetype which not available in subworld");
            }
        }
    }

    fn evaluate_query<'a>(
        &'a mut self,
        world: &StorageAccessor<'a>,
    ) -> (&mut Mutex<F>, QueryResult<'a>) {
        // pull layout matches out of the cache
        let cache = self.layout_matches.entry(world.id()).or_insert_with(|| {
            // if the query can match a group, look to see if there is a subgroup we can use
            let cache = if F::can_match_group() {
                let components = F::group_components();
                components
                    .get(0)
                    .and_then(|t| world.group(*t))
                    .map(|(i, g)| (i, g.exact_match(&components)))
                    .and_then(|(group, subgroup)| {
                        subgroup.map(|subgroup| Cache::Ordered { group, subgroup })
                    })
            } else {
                None
            };

            // else use an unordered result
            cache.unwrap_or_else(|| Cache::Unordered {
                archetypes: Vec::new(),
                seen: 0,
            })
        });

        // iteratively update our layout matches
        let filter = self.filter.get_mut();
        let result = match cache {
            Cache::Unordered { archetypes, seen } => {
                // resume index search from where we last left off
                for archetype in world.layout_index().search_from(&*filter, *seen) {
                    archetypes.push(archetype);
                }
                *seen = world.archetypes().len();
                QueryResult::unordered(archetypes.as_slice())
            }
            Cache::Ordered { group, subgroup } => {
                // grab the archetype slice from the group
                let archetypes = &world.groups()[*group][*subgroup];
                QueryResult::ordered(archetypes)
            }
        };

        Self::validate_archetype_access(world, result.index());

        (&mut self.filter, result)
    }

    pub(crate) fn find_archetypes<'a, T: EntityStore + 'a>(
        &'a mut self,
        world: &'a T,
    ) -> &'a [ArchetypeIndex] {
        let accessor = world.get_component_storage::<V::View>().unwrap();
        let (_, result) = self.evaluate_query(&accessor);
        result.index()
    }

    // ----------------
    // Entity Indexing
    // ----------------

    /// Returns the components for a single entity.
    ///
    /// This function will not evaluate the query's dynamic filters. This means, for example, that
    /// calling `get` on all entities in an archetype will not prevent `maybe_changed` from returning
    /// those entities the next time the query is iterated.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    pub unsafe fn get_unchecked<'query, 'world, T>(
        &'query mut self,
        world: &'world T,
        entity: Entity,
    ) -> Option<<V::View as View<'world>>::Element>
    where
        T: EntityStore,
    {
        let location = world.entry_ref(entity)?.location();
        let accessor = world.get_component_storage::<V::View>().unwrap();

        // safety:
        // This is much like the similar usage of transmute inside iter_chunks_unchecked, see
        // there for more details. In this case, the situation is simpler; we know the return
        // value of this function references only data from the world and View::fetch can't
        // put the reference away somewhere invalid (because the param is still bound to 'world not
        // 'static), so the arch_slice on the stack won't stil be in use after it falls out of scope.

        let arch_slice = [location.archetype()];
        let arch_slice_ref: &[ArchetypeIndex] = &arch_slice;
        let arch_slice_ref = std::mem::transmute(arch_slice_ref);
        let result = QueryResult::unordered(arch_slice_ref);
        let mut fetch =
            <V::View as View<'world>>::fetch(accessor.components(), accessor.archetypes(), result)
                .next()??;

        // if our filter has conditions beyond that of the view, then we need to evaluate the query
        if !self.is_view_filter {
            let (_, result) = self.evaluate_query(&accessor);
            if !result.index().contains(&location.archetype()) {
                return None;
            }
        }

        // accept the fetch to trigger version increments
        fetch.accepted();

        // construct a chunk view for the archetype, then index the entity we want
        let view = ChunkView::new(&accessor.archetypes()[location.archetype()], fetch);
        let mut iter = view.into_iter();
        use crate::internals::iter::indexed::TrustedRandomAccess;
        Some(iter.get_unchecked(location.component().0))
    }

    /// Returns the components for a single entity.
    pub fn get_mut<'query, 'world, T>(
        &'query mut self,
        world: &'world mut T,
        entity: Entity,
    ) -> Option<<V::View as View<'world>>::Element>
    where
        T: EntityStore,
    {
        // safety: we have exclusive access to world
        unsafe { self.get_unchecked(world, entity) }
    }

    /// Returns the components for a single entity.
    pub fn get<'query, 'world, T>(
        &'query mut self,
        world: &'world T,
        entity: Entity,
    ) -> Option<<V::View as View<'world>>::Element>
    where
        T: EntityStore,
        <V::View as View<'world>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.get_unchecked(world, entity) }
    }

    // ----------------
    // Chunk Iteration
    // ----------------

    /// Returns an iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    pub unsafe fn iter_chunks_unchecked<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world T,
    ) -> ChunkIter<'world, 'query, V::View, F> {
        let accessor = world.get_component_storage::<V::View>().unwrap();
        let (_, result) = self.evaluate_query(&accessor);

        // What we want:
        // Iterators returned from this function should yield component references
        // which are bound to the world, not the query, but the iterator itself
        // borrows data from both; while the iterator is alive, both the query and
        // world are borrowed, but it should be possible to drop the iterator and query
        // and keep those component references around as long as the world is alive.
        //
        // The ChunkIter keeps these two lifetimes separate. It yields component references
        // with the 'world lifetime, and stores internal state with 'query.
        //
        // The problem:
        // The index used by the iterator, depending on the state of the query, might be
        // borrowing data from *either* the query or the world. Therefore the two lifetimes
        // "flow into" each other. This confuses rustc as it cant pick a lifetime.
        //
        // Solving this with safe code would require two lifetime HRTBs on View/Fetch with
        // constraints between them. You can't do this.
        //
        // The workaround:
        // We transmute the lifetime of the index data into either 'world or 'query
        // depending on what is needed. This essentially creates a virtual 'index lifetime,
        // which is coerced into whatever is needed. This virtual lifetime is valid where
        // *both* 'query and 'world lifetimes are valid.
        //
        // We know 'query and 'world are both valid in this fn, as they are input lifetimes.
        //
        // The ChunkIter contains both lifetimes, so the returned iterator struct will extend
        // those lifetimes for at least as long as it is in scope.
        //
        // The ChunkIter only returns (unmolested) 'world references. Our virtual lifetime
        // cannot leak out of it. When the iterator is dropped, our virtual lifetime dies with it.

        let result = std::mem::transmute::<QueryResult<'_>, QueryResult<'world>>(result);
        let indices = std::mem::transmute::<Iter<'_, ArchetypeIndex>, Iter<'query, ArchetypeIndex>>(
            result.index.iter(),
        );

        let fetch =
            <V::View as View<'world>>::fetch(accessor.components(), accessor.archetypes(), result);
        let filter = self.filter.get_mut();
        filter.prepare(world.id());
        ChunkIter {
            inner: fetch,
            filter,
            archetypes: accessor.archetypes(),
            max_count: indices.len(),
            indices,
        }
    }

    /// Returns a parallel iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.    
    #[cfg(feature = "parallel")]
    pub unsafe fn par_iter_chunks_unchecked<'a, T: EntityStore>(
        &'a mut self,
        world: &'a T,
    ) -> par_iter::ParChunkIter<'a, V::View, F> {
        let accessor = world.get_component_storage::<V::View>().unwrap();
        let (filter, result) = self.evaluate_query(&accessor);
        par_iter::ParChunkIter::new(accessor, result, filter)
    }

    /// Returns an iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.      
    #[inline]
    pub fn iter_chunks_mut<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world mut T,
    ) -> ChunkIter<'world, 'query, V::View, F> {
        // safety: we have exclusive access to world
        unsafe { self.iter_chunks_unchecked(world) }
    }

    /// Returns a parallel iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_iter_chunks_mut<'a, T: EntityStore>(
        &'a mut self,
        world: &'a mut T,
    ) -> par_iter::ParChunkIter<'a, V::View, F> {
        // safety: we have exclusive access to world
        unsafe { self.par_iter_chunks_unchecked(world) }
    }

    /// Returns an iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    /// Only usable with queries who's views are read-only.
    #[inline]
    pub fn iter_chunks<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world T,
    ) -> ChunkIter<'world, 'query, V::View, F>
    where
        <V::View as View<'world>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.iter_chunks_unchecked(world) }
    }

    /// Returns a parallel iterator which will yield all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.
    /// Only usable with queries who's views are read-only.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_iter_chunks<'a, T: EntityStore>(
        &'a mut self,
        world: &'a T,
    ) -> par_iter::ParChunkIter<'a, V::View, F>
    where
        <V::View as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.par_iter_chunks_unchecked(world) }
    }

    // ----------------
    // Entity Iteration
    // ----------------

    /// Returns an iterator which will yield all components which match the query.
    ///
    /// Prefer `for_each_unchecked` as it offers better performance.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[inline]
    pub unsafe fn iter_unchecked<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world T,
    ) -> std::iter::Flatten<ChunkIter<'world, 'query, V::View, F>> {
        self.iter_chunks_unchecked(world).flatten()
    }

    /// Returns a parallel iterator which will yield all components which match the query.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[cfg(feature = "parallel")]
    #[inline]
    pub unsafe fn par_iter_unchecked<'a, T: EntityStore>(
        &'a mut self,
        world: &'a T,
    ) -> rayon::iter::Flatten<par_iter::ParChunkIter<'a, V::View, F>> {
        use rayon::iter::ParallelIterator;
        self.par_iter_chunks_unchecked(world).flatten()
    }

    /// Returns an iterator which will yield all components which match the query.
    ///
    /// Prefer `for_each_mut` as it yields better performance.
    #[inline]
    pub fn iter_mut<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world mut T,
    ) -> std::iter::Flatten<ChunkIter<'world, 'query, V::View, F>> {
        // safety: we have exclusive access to world
        unsafe { self.iter_unchecked(world) }
    }

    /// Returns a parallel iterator which will yield all components which match the query.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_iter_mut<'a, T: EntityStore>(
        &'a mut self,
        world: &'a mut T,
    ) -> rayon::iter::Flatten<par_iter::ParChunkIter<'a, V::View, F>> {
        // safety: we have exclusive access to world
        unsafe { self.par_iter_unchecked(world) }
    }

    /// Returns an iterator which will yield all components which match the query.
    ///
    /// Prefer `for_each` as it yields better performance.  
    /// Only usable with queries who's views are read-only.
    #[inline]
    pub fn iter<'query, 'world, T: EntityStore>(
        &'query mut self,
        world: &'world T,
    ) -> std::iter::Flatten<ChunkIter<'world, 'query, V::View, F>>
    where
        <V::View as View<'world>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.iter_unchecked(world) }
    }

    /// Returns a parallel iterator which will yield all components which match the query.
    ///
    /// Only usable with queries who's views are read-only.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_iter<'a, T: EntityStore>(
        &'a mut self,
        world: &'a T,
    ) -> rayon::iter::Flatten<par_iter::ParChunkIter<'a, V::View, F>>
    where
        <V::View as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.par_iter_unchecked(world) }
    }

    // ----------------
    // Chunk for-each
    // ----------------

    /// Iterates through all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[inline]
    pub unsafe fn for_each_chunk_unchecked<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world T,
        mut f: Body,
    ) where
        Body: FnMut(ChunkView<<V::View as View<'world>>::Fetch>),
    {
        for chunk in self.iter_chunks_unchecked(world) {
            f(chunk);
        }
    }

    /// Iterates in parallel through all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[cfg(feature = "parallel")]
    #[inline]
    pub unsafe fn par_for_each_chunk_unchecked<'a, T: EntityStore, Body>(
        &'a mut self,
        world: &'a T,
        f: Body,
    ) where
        Body: Fn(ChunkView<<V::View as View<'a>>::Fetch>) + Send + Sync,
    {
        use rayon::iter::ParallelIterator;
        self.par_iter_chunks_unchecked(world).for_each(f);
    }

    /// Iterates through all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    #[inline]
    pub fn for_each_chunk_mut<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world mut T,
        f: Body,
    ) where
        Body: FnMut(ChunkView<<V::View as View<'world>>::Fetch>),
    {
        // safety: we have exclusive access to world
        unsafe { self.for_each_chunk_unchecked(world, f) };
    }

    /// Iterates in parallel through all entity chunks which match the query.  
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_for_each_chunk_mut<'a, T: EntityStore, Body>(&'a mut self, world: &'a mut T, f: Body)
    where
        Body: Fn(ChunkView<<V::View as View<'a>>::Fetch>) + Send + Sync,
    {
        // safety: we have exclusive access to world
        unsafe { self.par_for_each_chunk_unchecked(world, f) };
    }

    /// Iterates through all entity chunks which match the query.  
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    /// Only usable with queries who's views are read-only.
    #[inline]
    pub fn for_each_chunk<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world T,
        f: Body,
    ) where
        Body: FnMut(ChunkView<<V::View as View<'world>>::Fetch>),
        <V::View as View<'world>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.for_each_chunk_unchecked(world, f) };
    }

    /// Iterates in parallel through all entity chunks which match the query.
    ///
    /// Each chunk contains slices of components for entities which all have the same component layout.  
    /// Only usable with queries who's views are read-only.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_for_each_chunk<'a, T: EntityStore, Body>(&'a mut self, world: &'a T, f: Body)
    where
        Body: Fn(ChunkView<<V::View as View<'a>>::Fetch>) + Send + Sync,
        <V::View as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.par_for_each_chunk_unchecked(world, f) };
    }

    // ----------------
    // Entity for-each
    // ----------------

    /// Iterates through all components which match the query.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[inline]
    pub unsafe fn for_each_unchecked<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world T,
        mut f: Body,
    ) where
        Body: FnMut(<V::View as View<'world>>::Element),
    {
        // we use a nested loop because it is significantly faster than .flatten()
        for chunk in self.iter_chunks_unchecked(world) {
            for entities in chunk {
                f(entities);
            }
        }
    }

    /// Iterates in parallel through all components which match the query.
    ///
    /// # Safety
    /// This function allows mutable access via a shared world reference. The caller is responsible for
    /// ensuring that no component accesses may create mutable aliases.
    #[cfg(feature = "parallel")]
    #[inline]
    pub unsafe fn par_for_each_unchecked<'a, T: EntityStore, Body>(
        &'a mut self,
        world: &'a T,
        f: Body,
    ) where
        Body: Fn(<V::View as View<'a>>::Element) + Send + Sync,
    {
        use rayon::iter::ParallelIterator;
        self.par_iter_unchecked(world).for_each(&f);
    }

    /// Iterates through all components which match the query.
    #[inline]
    pub fn for_each_mut<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world mut T,
        f: Body,
    ) where
        Body: FnMut(<V::View as View<'world>>::Element),
    {
        // safety: we have exclusive access to world
        unsafe { self.for_each_unchecked(world, f) };
    }

    /// Iterates in parallel through all components which match the query.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_for_each_mut<'a, T: EntityStore, Body>(&'a mut self, world: &'a mut T, f: Body)
    where
        Body: Fn(<V::View as View<'a>>::Element) + Send + Sync,
    {
        // safety: we have exclusive access to world
        unsafe { self.par_for_each_unchecked(world, f) };
    }

    /// Iterates through all components which match the query.
    ///
    /// Only usable with queries who's views are read-only.
    #[inline]
    pub fn for_each<'query, 'world, T: EntityStore, Body>(
        &'query mut self,
        world: &'world T,
        f: Body,
    ) where
        Body: FnMut(<V::View as View<'world>>::Element),
        <V::View as View<'world>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.for_each_unchecked(world, f) };
    }

    /// Iterates in parallel through all components which match the query.
    ///
    /// Only usable with queries who's views are read-only.
    #[cfg(feature = "parallel")]
    #[inline]
    pub fn par_for_each<'a, T: EntityStore, Body>(&'a mut self, world: &'a T, f: Body)
    where
        Body: Fn(<V::View as View<'a>>::Element) + Send + Sync,
        <V::View as View<'a>>::Fetch: ReadOnlyFetch,
    {
        // safety: the view is readonly - it cannot create mutable aliases
        unsafe { self.par_for_each_unchecked(world, f) };
    }
}

/// Provides access to slices of components for entities which have the same component layout.
///
/// A single index in any of the slices contained in a chunk belong to the same entity.
pub struct ChunkView<'a, F: Fetch> {
    archetype: &'a Archetype,
    fetch: F,
}

impl<'a, F: Fetch> ChunkView<'a, F> {
    fn new(archetype: &'a Archetype, fetch: F) -> Self {
        Self { archetype, fetch }
    }

    /// Returns the archetype that all entities in the chunk belong to.
    pub fn archetype(&self) -> &Archetype {
        &self.archetype
    }

    /// Returns a slice of components.
    ///
    /// May return `None` if the chunk's view does not declare access to the component type.
    pub fn component_slice<T: Component>(&self) -> Option<&[T]> {
        self.fetch.find::<T>()
    }

    /// Returns a mutable slice of components.
    ///
    /// May return `None` if the chunk's view does not declare access to the component type.
    pub fn component_slice_mut<T: Component>(&mut self) -> Option<&mut [T]> {
        self.fetch.find_mut::<T>()
    }

    /// Converts the chunk into a tuple of it's inner slices.
    ///
    /// # Examples
    ///
    /// ```
    /// # use legion::*;
    /// # struct A;
    /// # struct B;
    /// # struct C;
    /// # struct D;
    /// # let mut world = World::default();
    /// let mut query = <(Entity, Read<A>, Write<B>, TryRead<C>, TryWrite<D>)>::query();
    /// for chunk in query.iter_chunks_mut(&mut world) {
    ///     let slices: (&[Entity], &[A], &mut [B], Option<&[C]>, Option<&mut [D]>) = chunk.into_components();       
    /// }
    /// ```
    pub fn into_components(self) -> F::Data {
        self.fetch.into_components()
    }

    /// Converts the chunk into a tuple of it's inner slices.
    ///
    /// Only usable with views who's elements are all read-only.
    ///
    /// # Examples
    ///
    /// ```
    /// # use legion::*;
    /// # struct A;
    /// # struct B;
    /// # let mut world = World::default();
    /// let mut query = <(Entity, Read<A>, TryRead<B>)>::query();
    /// for chunk in query.iter_chunks_mut(&mut world) {
    ///     let slices: (&[Entity], &[A], Option<&[B]>) = chunk.get_components();       
    /// }
    /// ```
    pub fn get_components(&self) -> F::Data
    where
        F: ReadOnlyFetch,
    {
        self.fetch.get_components()
    }

    /// Converts the chunk into an iterator which yields tuples of `(Entity, components)`.
    pub fn into_iter_entities(
        self,
    ) -> impl Iterator<Item = (Entity, <F as IntoIndexableIter>::Item)> + 'a
    where
        <F as IntoIndexableIter>::IntoIter: 'a,
    {
        let iter = self.fetch.into_indexable_iter();
        self.archetype.entities().iter().copied().zip(iter)
    }
}

impl<'a, F: Fetch> IntoIterator for ChunkView<'a, F> {
    type IntoIter = <F as IntoIndexableIter>::IntoIter;
    type Item = <F as IntoIndexableIter>::Item;
    fn into_iter(self) -> Self::IntoIter {
        self.fetch.into_indexable_iter()
    }
}

#[cfg(feature = "parallel")]
impl<'a, F: Fetch> rayon::iter::IntoParallelIterator for ChunkView<'a, F> {
    type Iter = crate::internals::iter::indexed::par_iter::Par<<F as IntoIndexableIter>::IntoIter>;
    type Item = <<F as IntoIndexableIter>::IntoIter as crate::internals::iter::indexed::TrustedRandomAccess>::Item;
    fn into_par_iter(self) -> Self::Iter {
        use crate::internals::iter::indexed::par_iter::Par;
        Par::new(self.fetch.into_indexable_iter())
    }
}

/// An iterator which yields entity chunks from a query.
#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
pub struct ChunkIter<'data, 'index, V, D>
where
    V: View<'data>,
    D: DynamicFilter + 'index,
{
    inner: V::Iter,
    indices: Iter<'index, ArchetypeIndex>,
    filter: &'index mut D,
    archetypes: &'data [Archetype],
    max_count: usize,
}

impl<'world, 'query, V, D> Iterator for ChunkIter<'world, 'query, V, D>
where
    V: View<'world>,
    D: DynamicFilter + 'query,
{
    type Item = ChunkView<'world, V::Fetch>;

    fn next(&mut self) -> Option<Self::Item> {
        for fetch in &mut self.inner {
            // if fetch is None here, filtering is broken
            let mut fetch = fetch.unwrap();
            let idx = self.indices.next().unwrap();
            if self.filter.matches_archetype(&fetch).is_pass() {
                fetch.accepted();
                return Some(ChunkView::new(&self.archetypes[*idx], fetch));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.max_count))
    }
}

// impl<'world, 'query, I, F> Iterator for ChunkIter<'world, 'query, I, Passthrough, F>
// where
//     I: Iterator<Item = (ArchetypeIndex, F)>,
//     F: Fetch,
// {
//     type Item = ChunkView<'world, F>;

//     fn next(&mut self) -> Option<Self::Item> {
//         for (index, mut fetch) in &mut self.inner {
//             fetch.accepted();
//             return Some(ChunkView::new(&self.archetypes[index], fetch));
//         }
//         None
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) { (self.max_count, Some(self.max_count)) }
// }

// impl<'world, 'query, I, F> ExactSizeIterator for ChunkIter<'world, 'query, I, Passthrough, F>
// where
//     I: Iterator<Item = (ArchetypeIndex, F)> + ExactSizeIterator,
//     F: Fetch,
// {
// }

#[cfg(feature = "parallel")]
pub mod par_iter {
    use super::*;
    use rayon::iter::plumbing::{bridge_unindexed, Folder, UnindexedConsumer, UnindexedProducer};
    use rayon::iter::ParallelIterator;
    use std::marker::PhantomData;

    /// An entity chunk iterator which internally locks its filter during iteration.
    #[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
    pub struct Iter<'world, 'query, V, D>
    where
        V: View<'world>,
        D: DynamicFilter + 'query,
    {
        inner: V::Iter,
        indices: std::slice::Iter<'query, ArchetypeIndex>,
        filter: &'query Mutex<D>,
        archetypes: &'world [Archetype],
        max_count: usize,
    }

    impl<'world, 'query, V, D> Iterator for Iter<'world, 'query, V, D>
    where
        V: View<'world>,
        D: DynamicFilter + 'query,
    {
        type Item = ChunkView<'world, V::Fetch>;

        fn next(&mut self) -> Option<Self::Item> {
            let mut filter = self.filter.lock();
            for fetch in &mut self.inner {
                let mut fetch = fetch.unwrap();
                let idx = self.indices.next().unwrap();
                if filter.matches_archetype(&fetch).is_pass() {
                    fetch.accepted();
                    return Some(ChunkView::new(&self.archetypes[*idx], fetch));
                }
            }
            None
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, Some(self.max_count))
        }
    }

    /// A parallel entity chunk iterator.
    #[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
    pub struct ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
        world: StorageAccessor<'a>,
        result: QueryResult<'a>,
        filter: &'a Mutex<D>,
        _view: PhantomData<V>,
    }

    impl<'a, V, D> ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
        pub(super) fn new(
            world: StorageAccessor<'a>,
            result: QueryResult<'a>,
            filter: &'a Mutex<D>,
        ) -> Self {
            Self {
                world,
                result,
                filter,
                _view: PhantomData,
            }
        }
    }

    unsafe impl<'a, V, D> Send for ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
    }

    unsafe impl<'a, V, D> Sync for ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
    }

    impl<'a, V, D> UnindexedProducer for ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
        type Item = <Iter<'a, 'a, V, D> as Iterator>::Item;

        fn split(self) -> (Self, Option<Self>) {
            let index = self.result.len() / 2;
            let (left, right) = self.result.split_at(index);
            (
                Self {
                    world: self.world,
                    result: right,
                    filter: self.filter,
                    _view: PhantomData,
                },
                if !left.is_empty() {
                    Some(Self {
                        world: self.world,
                        result: left,
                        filter: self.filter,
                        _view: PhantomData,
                    })
                } else {
                    None
                },
            )
        }

        fn fold_with<F>(self, folder: F) -> F
        where
            F: Folder<Self::Item>,
        {
            let indices = self.result.index.iter();
            let fetch = unsafe {
                <V as View<'a>>::fetch(
                    self.world.components(),
                    self.world.archetypes(),
                    self.result,
                )
            };
            let iter = Iter::<'a, 'a, V, D> {
                inner: fetch,
                filter: self.filter,
                archetypes: self.world.archetypes(),
                max_count: indices.len(),
                indices,
            };
            folder.consume_iter(iter)
        }
    }

    impl<'a, V, D> ParallelIterator for ParChunkIter<'a, V, D>
    where
        V: View<'a>,
        D: DynamicFilter + 'a,
    {
        type Item = ChunkView<'a, V::Fetch>;

        fn drive_unindexed<C>(self, consumer: C) -> C::Result
        where
            C: UnindexedConsumer<Self::Item>,
        {
            bridge_unindexed(self, consumer)
        }
    }
}

#[cfg(test)]
mod test {
    use super::view::{read::Read, write::Write};
    use super::IntoQuery;
    use crate::internals::world::World;

    #[test]
    fn query() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);
        let entity = world.push((10usize, 5f32, false));

        let mut query = <(Read<usize>, Write<bool>)>::query();
        for (x, y) in query.iter_mut(&mut world) {
            println!("{}, {}", x, y);
        }
        for chunk in query.iter_chunks_mut(&mut world) {
            let (x, y) = chunk.into_components();
            println!("{:?}, {:?}", x, y);
        }

        #[cfg(feature = "parallel")]
        {
            query.par_for_each_mut(&mut world, |(x, y)| println!("{:?}, {:?}", x, y));
            use rayon::iter::ParallelIterator;
            query.par_iter_chunks_mut(&mut world).for_each(|chunk| {
                println!("arch {:?}", chunk.archetype());
                let (x, y) = chunk.into_components();
                println!("{:?}, {:?}", x, y);
            })
        }

        let single = query.get_mut(&mut world, entity);
        assert_eq!(single, Some((&10usize, &mut false)));
    }

    #[test]
    fn query_split() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);

        let mut query_a = Write::<usize>::query();
        let mut query_b = Write::<bool>::query();

        let (mut left, mut right) = world.split::<Write<usize>>();

        for x in query_a.iter_mut(&mut left) {
            println!("{:}", x);
        }

        for x in query_b.iter_mut(&mut right) {
            println!("{:}", x);
        }
    }

    #[test]
    #[should_panic]
    fn query_split_disallowd_component_left() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);

        let mut query_a = Write::<usize>::query();
        let mut query_b = Write::<bool>::query();

        let (mut left, _) = world.split::<Write<usize>>();

        for x in query_a.iter_mut(&mut left) {
            println!("{:}", x);
        }

        for x in query_b.iter_mut(&mut left) {
            println!("{:}", x);
        }
    }

    #[test]
    #[should_panic]
    fn query_split_disallowd_component_right() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);

        let mut query_a = Write::<usize>::query();
        let mut query_b = Write::<bool>::query();

        let (_, mut right) = world.split::<Write<usize>>();

        for x in query_a.iter_mut(&mut right) {
            println!("{:}", x);
        }

        for x in query_b.iter_mut(&mut right) {
            println!("{:}", x);
        }
    }

    #[test]
    fn query_component_lifetime() {
        let mut world = World::default();
        world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);

        let mut components: Vec<&usize> = Vec::new();

        {
            let mut query_a = Read::<usize>::query();
            components.extend(query_a.iter(&world));
        }

        assert_eq!(components[0], &1usize);
    }
}
