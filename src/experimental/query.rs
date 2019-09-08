use crate::experimental::borrow::Exclusive;
use crate::experimental::borrow::RefIter;
use crate::experimental::borrow::RefIterMut;
use crate::experimental::borrow::RefMap;
use crate::experimental::borrow::RefMapMut;
use crate::experimental::borrow::Shared;
use crate::experimental::entity::Entity;
use crate::experimental::filter::And;
use crate::experimental::filter::ArchetypeFilterData;
use crate::experimental::filter::ChunkFilterData;
use crate::experimental::filter::ComponentFilter;
use crate::experimental::filter::EntityFilter;
use crate::experimental::filter::EntityFilterTuple;
use crate::experimental::filter::Filter;
use crate::experimental::filter::FilterResult;
use crate::experimental::filter::Passthrough;
use crate::experimental::filter::TagFilter;
use crate::experimental::storage::ArchetypeData;
use crate::experimental::storage::Component;
use crate::experimental::storage::ComponentStorage;
use crate::experimental::storage::ComponentTypeId;
use crate::experimental::storage::Storage;
use crate::experimental::storage::Tag;
use crate::experimental::storage::TagTypeId;
use crate::experimental::world::World;
use std::any::TypeId;
use std::iter::Enumerate;
use std::iter::Repeat;
use std::iter::Take;
use std::marker::PhantomData;
use std::slice::Iter;
use std::slice::IterMut;

#[cfg(feature = "par-iter")]
use rayon::prelude::*;

/// A type which can fetch a strongly-typed view of the data contained
/// within a chunk.
pub trait View<'a>: Sized + Send + Sync + 'static {
    /// The iterator over the chunk data.
    type Iter: Iterator + 'a;

    /// Pulls data out of a chunk.
    fn fetch(
        archetype: &'a ArchetypeData,
        chunk: &'a ComponentStorage,
        chunk_index: usize,
    ) -> Self::Iter;

    /// Validates that the view does not break any component borrowing rules.
    fn validate() -> bool;

    /// Determines if the view reads the specified data type.
    fn reads<T: Component>() -> bool;

    /// Determines if the view writes to the specified data type.
    fn writes<T: Component>() -> bool;
}

/// A type which can construct a default entity filter.
pub trait DefaultFilter {
    /// The type of entity filter constructed.
    type Filter: EntityFilter;

    /// constructs an entity filter.
    fn filter() -> Self::Filter;
}

#[doc(hidden)]
pub trait ViewElement {
    type Component;
}

/// Converts a `View` into a `Query`.
pub trait IntoQuery: DefaultFilter + for<'a> View<'a> {
    /// Converts the `View` type into a `Query`.
    fn query() -> Query<Self, <Self as DefaultFilter>::Filter>;
}

impl<T: DefaultFilter + for<'a> View<'a>> IntoQuery for T {
    fn query() -> Query<Self, <Self as DefaultFilter>::Filter> {
        if !Self::validate() {
            panic!("invalid view, please ensure the view contains no duplicate component types");
        }

        Query {
            view: PhantomData,
            filter: Self::filter(),
        }
    }
}

/// Reads a single entity data component type from a chunk.
#[derive(Debug)]
pub struct Read<T: Component>(PhantomData<T>);

impl<'a, T: Component> DefaultFilter for Read<T> {
    type Filter = EntityFilterTuple<ComponentFilter<T>, Passthrough>;

    fn filter() -> Self::Filter { super::filter::filter_fns::component() }
}

impl<'a, T: Component> View<'a> for Read<T> {
    type Iter = RefIter<'a, Shared<'a>, T, Iter<'a, T>>;

    fn fetch(_: &'a ArchetypeData, chunk: &'a ComponentStorage, _: usize) -> Self::Iter {
        let (slice_borrow, slice) = unsafe {
            chunk
                .components(ComponentTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .deconstruct()
        };
        RefIter::new(slice_borrow, slice.iter())
    }

    fn validate() -> bool { true }

    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    fn writes<D: Component>() -> bool { false }
}

impl<T: Component> ViewElement for Read<T> {
    type Component = T;
}

/// Writes to a single entity data component type from a chunk.
#[derive(Debug)]
pub struct Write<T: Component>(PhantomData<T>);

impl<'a, T: Component> DefaultFilter for Write<T> {
    type Filter = EntityFilterTuple<ComponentFilter<T>, Passthrough>;

    fn filter() -> Self::Filter { super::filter::filter_fns::component() }
}

impl<'a, T: Component> View<'a> for Write<T> {
    type Iter = RefIterMut<'a, Exclusive<'a>, T, IterMut<'a, T>>;

    #[inline]
    fn fetch(_: &'a ArchetypeData, chunk: &'a ComponentStorage, _: usize) -> Self::Iter {
        let (slice_borrow, slice) = unsafe {
            chunk
                .components(ComponentTypeId::of::<T>())
                .unwrap()
                .data_slice_mut::<T>()
                .deconstruct()
        };
        RefIterMut::new(slice_borrow, slice.iter_mut())
    }

    #[inline]
    fn validate() -> bool { true }

    #[inline]
    fn reads<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }

    #[inline]
    fn writes<D: Component>() -> bool { TypeId::of::<T>() == TypeId::of::<D>() }
}

impl<T: Component> ViewElement for Write<T> {
    type Component = T;
}

/// Reads a single shared data component type in a chunk.
#[derive(Debug)]
pub struct Tagged<T: Tag>(PhantomData<T>);

impl<'a, T: Tag> DefaultFilter for Tagged<T> {
    type Filter = EntityFilterTuple<TagFilter<T>, Passthrough>;

    fn filter() -> Self::Filter { super::filter::filter_fns::tag() }
}

impl<'a, T: Tag> View<'a> for Tagged<T> {
    type Iter = Take<Repeat<&'a T>>;

    #[inline]
    fn fetch(
        archetype: &'a ArchetypeData,
        chunk: &'a ComponentStorage,
        chunk_index: usize,
    ) -> Self::Iter {
        let data = unsafe {
            archetype
                .tags(TagTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .get_unchecked(chunk_index)
        };
        std::iter::repeat(data).take(chunk.len())
    }

    #[inline]
    fn validate() -> bool { true }

    #[inline]
    fn reads<D: Component>() -> bool { false }

    #[inline]
    fn writes<D: Component>() -> bool { false }
}

impl<T: Tag> ViewElement for Tagged<T> {
    type Component = Tagged<T>;
}

macro_rules! impl_view_tuple {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: ViewElement + DefaultFilter ),*> DefaultFilter for ($( $ty, )*) {
            type Filter = EntityFilterTuple<And<($( <$ty::Filter as EntityFilter>::ArchetypeFilter, )*)>, And<($( <$ty::Filter as EntityFilter>::ChunkFilter, )*)>>;

            fn filter() -> Self::Filter {
                #![allow(non_snake_case)]
                $( let $ty = $ty::filter().into_filters(); )*
                EntityFilterTuple::new(
                    And { filters: ($( $ty.0, )*) },
                    And { filters: ($( $ty.1, )*) },
                )
            }
        }

        impl<'a, $( $ty: ViewElement + View<'a> ),* > View<'a> for ($( $ty, )*) {
            type Iter = itertools::Zip<($( $ty::Iter, )*)>;

            #[inline]
            fn fetch(
                archetype: &'a ArchetypeData,
                chunk: &'a ComponentStorage,
                chunk_index: usize,
            ) -> Self::Iter {
                itertools::multizip(($( $ty::fetch(archetype.clone(), chunk.clone(), chunk_index), )*))
            }

            fn validate() -> bool {
                let types = &[$( TypeId::of::<$ty::Component>() ),*];
                for i in 0..types.len() {
                    for j in (i + 1)..types.len() {
                        if unsafe { types.get_unchecked(i) == types.get_unchecked(j) } {
                            return false;
                        }
                    }
                }

                true
            }

            fn reads<Data: Component>() -> bool {
                $( $ty::reads::<Data>() )||*
            }

            fn writes<Data: Component>() -> bool {
                $( $ty::reads::<Data>() )||*
            }
        }
    };
}

impl_view_tuple!(A);
impl_view_tuple!(A, B);
impl_view_tuple!(A, B, C);
impl_view_tuple!(A, B, C, D);
impl_view_tuple!(A, B, C, D, E);
impl_view_tuple!(A, B, C, D, E, F);

/// A type-safe view of a chunk of entities all of the same data layout.
pub struct Chunk<'a, V: for<'b> View<'b>> {
    archetype: &'a ArchetypeData,
    components: &'a ComponentStorage,
    index: usize,
    view: PhantomData<V>,
}

impl<'a, V: for<'b> View<'b>> Chunk<'a, V> {
    pub fn new(archetype: &'a ArchetypeData, index: usize) -> Self {
        Self {
            components: archetype.component_chunk(index).unwrap(),
            archetype,
            index,
            view: PhantomData,
        }
    }

    /// Get a slice of all entities contained within the chunk.
    #[inline]
    pub fn entities(&self) -> &'a [Entity] { self.components.entities() }

    /// Get an iterator of all data contained within the chunk.
    #[inline]
    pub fn iter(&mut self) -> <V as View<'a>>::Iter {
        V::fetch(self.archetype, self.components, self.index)
    }

    /// Get an iterator of all data and entity IDs contained within the chunk.
    #[inline]
    pub fn iter_entities(&mut self) -> ZipEntities<'a, V> {
        ZipEntities {
            entities: self.entities(),
            data: V::fetch(self.archetype, self.components, self.index),
            index: 0,
            view: PhantomData,
        }
    }

    /// Get a tag value.
    pub fn tag<T: Tag>(&self) -> Option<&T> {
        self.archetype
            .tags(TagTypeId::of::<T>())
            .map(|tags| unsafe { tags.data_slice::<T>() })
            .map(|slice| unsafe { slice.get_unchecked(self.index) })
    }

    /// Get a slice of component data.
    ///
    /// # Panics
    ///
    /// This method performs runtime borrow checking. It will panic if
    /// any other code is concurrently writing to the data slice.
    pub fn components<T: Component>(&self) -> Option<RefMap<'a, Shared<'a>, &[T]>> {
        if !V::reads::<T>() {
            panic!("data type not readable via this query");
        }
        self.components
            .components(ComponentTypeId::of::<T>())
            .map(|c| unsafe { c.data_slice::<T>() })
    }

    /// Get a mutable slice of component data.
    ///
    /// # Panics
    ///
    /// This method performs runtime borrow checking. It will panic if
    /// any other code is concurrently accessing the data slice.
    pub fn components_mut<T: Component>(&self) -> Option<RefMapMut<'a, Exclusive<'a>, &mut [T]>> {
        if !V::writes::<T>() {
            panic!("data type not writable via this query");
        }
        self.components
            .components(ComponentTypeId::of::<T>())
            .map(|c| unsafe { c.data_slice_mut::<T>() })
    }
}

/// An iterator which yields view data tuples and entity IDs from a `Chunk`.
pub struct ZipEntities<'data, V: View<'data>> {
    entities: &'data [Entity],
    data: <V as View<'data>>::Iter,
    index: usize,
    view: PhantomData<V>,
}

impl<'data, V: View<'data>> Iterator for ZipEntities<'data, V> {
    type Item = (Entity, <V::Iter as Iterator>::Item);

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

/// An iterator over all chunks that match a given query.
pub struct ChunkViewIter<
    'data,
    'filter,
    V: for<'a> View<'a>,
    Arch: Filter<ArchetypeFilterData<'data>>,
    Chunk: Filter<ChunkFilterData<'data>>,
> {
    _view: PhantomData<V>,
    storage: &'data Storage,
    arch_filter: &'filter mut Arch,
    chunk_filter: &'filter mut Chunk,
    archetypes: Enumerate<Arch::Iter>,
    frontier: Option<(&'data ArchetypeData, Take<Enumerate<Chunk::Iter>>)>,
}

impl<
        'data,
        'filter,
        V: for<'a> View<'a>,
        ArchFilter: Filter<ArchetypeFilterData<'data>>,
        ChunkFilter: Filter<ChunkFilterData<'data>>,
    > Iterator for ChunkViewIter<'data, 'filter, V, ArchFilter, ChunkFilter>
{
    type Item = Chunk<'data, V>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((ref arch, ref mut chunks)) = self.frontier {
                for (chunk_index, chunk_data) in chunks {
                    if self.chunk_filter.is_match(&chunk_data).is_pass() {
                        return Some(Chunk::new(arch, chunk_index));
                    }
                }
            }
            loop {
                match self.archetypes.next() {
                    Some((arch_index, arch_data)) => {
                        if self.arch_filter.is_match(&arch_data).is_pass() {
                            self.frontier = {
                                let chunks = unsafe { self.storage.data_unchecked(arch_index) };
                                let data = ChunkFilterData {
                                    archetype_data: chunks,
                                };

                                Some((
                                    chunks,
                                    self.chunk_filter
                                        .collect(data)
                                        .enumerate()
                                        .take(chunks.len()),
                                ))
                            };
                            break;
                        }
                    }
                    None => return None,
                }
            }
        }
    }
}

/// An iterator which iterates through all entity data in all chunks.
pub struct ChunkDataIter<'data, V, I>
where
    V: for<'a> View<'a>,
    I: Iterator<Item = Chunk<'data, V>>,
{
    iter: I,
    frontier: Option<<V as View<'data>>::Iter>,
    _view: PhantomData<V>,
}

impl<'data, V, I> Iterator for ChunkDataIter<'data, V, I>
where
    V: for<'a> View<'a>,
    I: Iterator<Item = Chunk<'data, V>>,
{
    type Item = <<V as View<'data>>::Iter as Iterator>::Item;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut inner) = self.frontier {
                if let elt @ Some(_) = inner.next() {
                    return elt;
                }
            }
            match self.iter.next() {
                Some(mut inner) => self.frontier = Some(inner.iter()),
                None => return None,
            }
        }
    }
}

/// An iterator which iterates through all entity data in all chunks, zipped with entity ID.
pub struct ChunkEntityIter<'data, V, I>
where
    V: for<'a> View<'a>,
    I: Iterator<Item = Chunk<'data, V>>,
{
    iter: I,
    frontier: Option<ZipEntities<'data, V>>,
    _view: PhantomData<V>,
}

impl<'data, 'query, V, I> Iterator for ChunkEntityIter<'data, V, I>
where
    V: for<'a> View<'a>,
    I: Iterator<Item = Chunk<'data, V>>,
{
    type Item = (Entity, <<V as View<'data>>::Iter as Iterator>::Item);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut inner) = self.frontier {
                if let elt @ Some(_) = inner.next() {
                    return elt;
                }
            }
            match self.iter.next() {
                Some(mut inner) => self.frontier = Some(inner.iter_entities()),
                None => return None,
            }
        }
    }
}

/// Queries for entities within a `World`.
///
/// # Examples
///
/// Queries can be constructed from any `View` type, including tuples of `View`s.
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// // A query which matches any entity with a `Position` component
/// let mut query = Read::<Position>::query();
///
/// // A query which matches any entity with both a `Position` and a `Velocity` component
/// let mut query = <(Read<Position>, Read<Velocity>)>::query();
/// ```
///
/// The view determines what data is accessed, and whether it is accessed mutably or not.
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// // A query which writes `Position`, reads `Velocity` and reads `Model`
/// // Tags are read-only, and is distinguished from entity data reads with `Tagged<T>`.
/// let mut query = <(Write<Position>, Read<Velocity>, Tagged<Model>)>::query();
/// ```
///
/// By default, a query will filter its results to include only entities with the data
/// types accessed by the view. However, additional filters can be specified if needed:
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// #[derive(Copy, Clone, Debug, PartialEq)]
/// struct Static;
///
/// // A query which also requires that entities have the `Static` tag
/// let mut query = <(Read<Position>, Tagged<Model>)>::query().filter(tag::<Static>());
/// ```
///
/// Filters can be combined with bitwise operators:
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// #[derive(Copy, Clone, Debug, PartialEq)]
/// struct Static;
///
/// // This query matches entities with positions and a model
/// // But it also requires that the entity is not static, or has moved (even if static)
/// let mut query = <(Read<Position>, Tagged<Model>)>::query()
///     .filter(!tag::<Static>() | changed::<Position>());
/// ```
///
/// Filters can be iterated through to pull data out of a `World`:
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// # let universe = Universe::new(None);
/// # let world = universe.create_world();
/// // A query which writes `Position`, reads `Velocity` and reads `Model`
/// // Tags are read-only, and is distinguished from entity data reads with `Tagged<T>`.
/// let mut query = <(Write<Position>, Read<Velocity>, Tagged<Model>)>::query();
///
/// for (mut pos, vel, model) in query.iter(&world) {
///     // `.iter` yields tuples of references to a single entity's data:
///     // pos: &mut Position
///     // vel: &Velocity
///     // model: &Model
/// }
/// ```
///
/// The lower level `iter_chunks` function allows access to each underlying chunk of entity data.
/// This allows you to run code for each tag value, or to retrieve a contiguous data slice.
///
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// # let universe = Universe::new(None);
/// # let world = universe.create_world();
/// let mut query = <(Write<Position>, Read<Velocity>, Tagged<Model>)>::query();
///
/// for chunk in query.iter_chunks(&world) {
///     let model = chunk.tag::<Model>();
///     let positions = chunk.components_mut::<Position>();
///     let velocities = chunk.components::<Velocity>();
/// }
/// ```
///
/// The `ChunkView` yielded from `iter_chunks` allows access to all shared data in the chunk (queried for or not),
/// but entity data slices can only be accessed if they were requested in the query's view. Attempting to access
/// other data types, or attempting to write to components that were only requested via a `Read` will panic.
pub struct Query<V: for<'a> View<'a>, F: EntityFilter> {
    view: PhantomData<V>,
    filter: F,
}

impl<V, F> Query<V, F>
where
    V: for<'a> View<'a>,
    F: EntityFilter,
{
    /// Adds an additional filter to the query.
    pub fn filter<T: EntityFilter>(self, filter: T) -> Query<V, <F as std::ops::BitAnd<T>>::Output>
    where
        F: std::ops::BitAnd<T>,
        <F as std::ops::BitAnd<T>>::Output: EntityFilter,
    {
        Query {
            view: self.view,
            filter: self.filter & filter,
        }
    }

    /// Gets an iterator which iterates through all chunks that match the query.
    pub fn iter_chunks<'a, 'data>(
        &'a mut self,
        world: &'data World,
    ) -> ChunkViewIter<'data, 'a, V, F::ArchetypeFilter, F::ChunkFilter> {
        let (arch_filter, chunk_filter) = self.filter.filters();
        let storage = &world.archetypes;
        let archetypes = arch_filter
            .collect(ArchetypeFilterData {
                component_types: storage.component_types(),
                tag_types: storage.tag_types(),
            })
            .enumerate();
        ChunkViewIter {
            storage,
            arch_filter,
            chunk_filter,
            archetypes,
            frontier: None,
            _view: PhantomData,
        }
    }

    /// Gets an iterator which iterates through all entity data that matches the query, and also yields the the `Entity` IDs.
    pub fn iter_entities<'a, 'data>(
        &'a mut self,
        world: &'data World,
    ) -> ChunkEntityIter<'data, V, ChunkViewIter<'data, 'a, V, F::ArchetypeFilter, F::ChunkFilter>>
    {
        ChunkEntityIter {
            iter: self.iter_chunks(world),
            frontier: None,
            _view: PhantomData,
        }
    }

    /// Gets an iterator which iterates through all entity data that matches the query.
    pub fn iter<'a, 'data>(
        &'a mut self,
        world: &'data World,
    ) -> ChunkDataIter<'data, V, ChunkViewIter<'data, 'a, V, F::ArchetypeFilter, F::ChunkFilter>>
    {
        ChunkDataIter {
            iter: self.iter_chunks(world),
            frontier: None,
            _view: PhantomData,
        }
    }

    /// Iterates through all entity data that matches the query.
    pub fn for_each<'a, 'data, T>(&'a mut self, world: &'data World, mut f: T)
    where
        T: Fn(<<V as View<'data>>::Iter as Iterator>::Item),
    {
        self.iter(world).for_each(&mut f);
    }

    /// Iterates through all entity data that matches the query in parallel.
    #[cfg(feature = "par-iter")]
    pub fn par_for_each<'a, T>(&'a mut self, world: &'a World, f: T)
    where
        T: Fn(<<V as View<'a>>::Iter as Iterator>::Item) + Send + Sync,
    {
        self.par_iter_chunks(world).for_each(|mut chunk| {
            for data in chunk.iter() {
                f(data);
            }
        });
    }

    /// Gets a parallel iterator of chunks that match the query.
    #[cfg(feature = "par-iter")]
    pub fn par_iter_chunks<'a>(
        &'a mut self,
        world: &'a World,
    ) -> impl ParallelIterator<Item = Chunk<'a, V>> {
        self.iter_chunks(world).par_bridge()
    }
}
