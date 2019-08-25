use crate::experimental::borrow::Borrow;
use crate::experimental::borrow::Ref;
use crate::experimental::borrow::RefMap;
use crate::experimental::borrow::RefMapMut;
use crate::experimental::entity::Entity;
use crate::experimental::filter::And;
use crate::experimental::filter::ArchetypeFilterData;
use crate::experimental::filter::ChunkFilterData;
use crate::experimental::filter::EntityFilter;
use crate::experimental::filter::Filter;
use crate::experimental::filter::FilterEntityIter;
use crate::experimental::storage::ArchetypeData;
use crate::experimental::storage::ArchetypeId;
use crate::experimental::storage::ChunkId;
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
use std::ops::Deref;
use std::slice::Iter;
use std::slice::IterMut;

/// A type which can fetch a strongly-typed view of the data contained
/// within a `Chunk`.
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

#[doc(hidden)]
pub trait ViewElement {
    type Component;
}

/// Reads a single entity data component type from a `Chunk`.
#[derive(Debug)]
pub struct Read<T: Component>(PhantomData<T>);

impl<'a, T: Component> View<'a> for Read<T> {
    type Iter = RefMap<'a, Iter<'a, T>>;

    fn fetch(_: &'a ArchetypeData, chunk: &'a ComponentStorage, _: usize) -> Self::Iter {
        unsafe {
            chunk
                .components(&ComponentTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .map_into(|x| x.iter())
        }
    }

    fn validate() -> bool {
        true
    }

    fn reads<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    fn writes<D: Component>() -> bool {
        false
    }
}

impl<T: Component> ViewElement for Read<T> {
    type Component = T;
}

/// Writes to a single entity data component type from a `Chunk`.
#[derive(Debug)]
pub struct Write<T: Component>(PhantomData<T>);

impl<'a, T: Component> View<'a> for Write<T> {
    type Iter = RefMapMut<'a, IterMut<'a, T>>;

    fn fetch(_: &'a ArchetypeData, chunk: &'a ComponentStorage, _: usize) -> Self::Iter {
        unsafe {
            let (borrow, slice) = chunk
                .components(&ComponentTypeId::of::<T>())
                .unwrap()
                .data_slice_mut::<T>()
                .deconstruct();

            RefMapMut::new(borrow, slice.iter_mut())
        }
    }

    fn validate() -> bool {
        true
    }

    fn reads<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    fn writes<D: Component>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }
}

impl<T: Component> ViewElement for Write<T> {
    type Component = T;
}

/// Reads a single shared data component type in a `Chunk`.
#[derive(Debug)]
pub struct Tagged<T: Tag>(PhantomData<T>);

impl<'a, T: Tag> View<'a> for Tagged<T> {
    type Iter = Take<Repeat<&'a T>>;

    fn fetch(
        archetype: &'a ArchetypeData,
        chunk: &'a ComponentStorage,
        chunk_index: usize,
    ) -> Self::Iter {
        let data = unsafe {
            archetype
                .tags(&TagTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .get_unchecked(chunk_index)
        };
        std::iter::repeat(data).take(chunk.len())
    }

    fn validate() -> bool {
        true
    }

    fn reads<D: Component>() -> bool {
        false
    }

    fn writes<D: Component>() -> bool {
        false
    }
}

impl<T: Tag> ViewElement for Tagged<T> {
    type Component = Tagged<T>;
}

macro_rules! impl_view_tuple {
    ( $( $ty: ident ),* ) => {
        impl<'a, $( $ty: ViewElement + View<'a> ),* > View<'a> for ($( $ty, )*) {
            type Iter = itertools::Zip<($( $ty::Iter, )*)>;

            fn fetch(
                archetype: &'a ArchetypeData,
                chunk: &'a ComponentStorage,
                chunk_index: usize,
            ) -> Self::Iter {
                itertools::multizip(($( $ty::fetch(archetype, chunk, chunk_index), )*))
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

/// A type-safe view of a "chunk" of entities.
pub struct Chunk<'a, V: for<'b> View<'b>> {
    archetype: Ref<'a, ArchetypeData>,
    components: &'a ComponentStorage,
    index: usize,
    view: PhantomData<V>,
}

impl<'a, V: for<'b> View<'b>> Chunk<'a, V> {
    pub fn new(archetype: Ref<'a, ArchetypeData>, index: usize) -> Self {
        let (borrow, arch) = unsafe { archetype.deconstruct() };
        Self {
            archetype: Ref::new(borrow, arch),
            components: arch.component_chunk(index).unwrap(),
            index,
            view: PhantomData,
        }
    }

    /// Get a slice of all entities contained within the chunk.
    #[inline]
    pub fn entities(&self) -> &'a [Entity] {
        self.components.entities()
    }

    /// Get an iterator of all data contained within the chunk.
    #[inline]
    pub fn iter<'b: 'a>(&'b mut self) -> <V as View<'b>>::Iter {
        V::fetch(self.archetype.deref(), self.components, self.index)
    }

    /// Get an iterator of all data and entity IDs contained within the chunk.
    #[inline]
    pub fn iter_entities<'b: 'a>(&'b mut self) -> ZipEntities<'b, V> {
        ZipEntities {
            entities: self.entities(),
            data: V::fetch(self.archetype.deref(), self.components, self.index),
            index: 0,
            view: PhantomData,
        }
    }

    /// Get a tag value.
    pub fn tag<T: Tag>(&self) -> Option<&T> {
        self.archetype
            .tags(&TagTypeId::of::<T>())
            .map(|tags| unsafe { tags.data_slice::<T>() })
            .map(|slice| unsafe { slice.get_unchecked(self.index) })
    }

    /// Get a slice of component data.
    ///
    /// # Panics
    ///
    /// This method performs runtime borrow checking. It will panic if
    /// any other code is concurrently writing to the data slice.
    pub fn components<T: Component>(&self) -> Option<RefMap<'a, &[T]>> {
        if !V::reads::<T>() {
            panic!("data type not readable via this query");
        }
        unsafe {
            self.components
                .components(&ComponentTypeId::of::<T>())
                .map(|components| components.data_slice::<T>())
        }
    }

    /// Get a mutable slice of component data.
    ///
    /// # Panics
    ///
    /// This method performs runtime borrow checking. It will panic if
    /// any other code is concurrently accessing the data slice.
    pub fn components_mut<T: Component>(&self) -> Option<RefMapMut<'a, &mut [T]>> {
        if !V::writes::<T>() {
            panic!("data type not writable via this query");
        }
        unsafe {
            self.components
                .components(&ComponentTypeId::of::<T>())
                .map(|components| components.data_slice_mut::<T>())
        }
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

pub struct ChunkViewIter<'data, V: for<'a> View<'a>, I: Iterator<Item = ChunkId>> {
    iter: I,
    storage: &'data Storage,
    current_archetype: Option<(ArchetypeId, Ref<'data, ArchetypeData>)>,
    view: PhantomData<V>,
}

impl<'data, V: for<'a> View<'a>, I: Iterator<Item = ChunkId>> Iterator
    for ChunkViewIter<'data, V, I>
{
    type Item = Chunk<'data, V>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(chunk_id) = self.iter.next() {
            if self
                .current_archetype
                .as_ref()
                .map_or(true, |(archetype_id, _)| {
                    *archetype_id != chunk_id.archetype_id()
                })
            {
                let id = chunk_id.archetype_id();
                let arch = self.storage.data(id.index()).unwrap().deref().get();
                self.current_archetype = Some((id, arch));
            }

            if let Some((_, archetype)) = &self.current_archetype {
                return Some(Chunk::new(archetype.clone(), chunk_id.index()));
            }
        }

        None
    }
}

pub struct ChunkViewIter2<
    'data,
    'filter,
    Arch: Filter<'data, ArchetypeFilterData<'data>>,
    Chunk: Filter<'data, ChunkFilterData<'data>>,
> {
    storage: &'data Storage,
    arch_filter: &'filter mut Arch,
    chunk_filter: &'filter mut Chunk,
    archetypes: Enumerate<Arch::Iter>,
    frontier: Option<(
        ArchetypeId,
        &'data ArchetypeData,
        Borrow<'data>,
        Enumerate<Chunk::Iter>,
    )>,
}

// impl<
//         'data,
//         'filter,
//         Arch: Filter<'data, ArchetypeFilterData<'data>>,
//         Chunk: Filter<'data, ChunkFilterData<'data>>,
//     > Iterator for ChunkViewIter2<'data, 'filter, Arch, Chunk>
// {
// }

pub struct Query<V: for<'a> View<'a>, F: EntityFilter> {
    view: PhantomData<V>,
    filter: F,
}

impl<V, F> Query<V, F>
where
    V: for<'a> View<'a>,
    F: EntityFilter,
{
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

    // fn iter_chunks<'a, 'data>(
    //     &'a mut self,
    //     world: &'data World,
    // ) -> ChunkViewIter<'data, V, FilterEntityIter<'a, 'data, F::ArchetypeFilter, F::ChunkFilter>>
    // {
    //     ChunkViewIter {
    //         iter: self.filter.iter(&world.archetypes),
    //         storage: &world.archetypes,
    //         current_archetype: None,
    //         view: PhantomData,
    //     }
    // }
}
