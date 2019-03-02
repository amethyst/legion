use itertools::multizip;
use std::iter::Repeat;
use std::iter::Take;
use std::marker::PhantomData;
use std::slice::Iter;
use std::slice::IterMut;

use crate::*;

pub trait View<'a>: Sized + 'static {
    type Iter: Iterator + 'a;
    type Filter: ArchetypeFilter;

    fn fetch(chunk: &'a Chunk) -> Self::Iter;
    fn filter() -> Self::Filter;
    fn validate() -> bool;
}

pub trait ViewElement {
    type Component;
}

pub trait Queryable<'a, World>: View<'a> {
    fn query(world: World) -> Query<'a, Self, <Self as View<'a>>::Filter, Passthrough>;
}

impl<'a, T: View<'a>> Queryable<'a, &'a mut World> for T {
    fn query(world: &'a mut World) -> Query<'a, Self, Self::Filter, Passthrough> {
        if !Self::validate() {
            panic!("invalid view, please ensure the view contains no duplicate component types");
        }

        Query {
            world: world,
            view: PhantomData,
            arch_filter: Self::filter(),
            chunk_filter: Passthrough,
        }
    }
}

pub trait ReadOnly {}

impl<'a, T: View<'a> + ReadOnly> Queryable<'a, &'a World> for T {
    fn query(world: &'a World) -> Query<'a, Self, Self::Filter, Passthrough> {
        if !Self::validate() {
            panic!("invalid view, please ensure the view contains no duplicate component types");
        }

        Query {
            world,
            view: PhantomData,
            arch_filter: Self::filter(),
            chunk_filter: Passthrough,
        }
    }
}

#[derive(Debug)]
pub struct Read<T: EntityData>(PhantomData<T>);

impl<T: EntityData> ReadOnly for Read<T> {}

impl<'a, T: EntityData> View<'a> for Read<T> {
    type Iter = BorrowedIter<'a, Iter<'a, T>>;
    type Filter = EntityDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        chunk.entity_data().unwrap().into_iter()
    }

    fn filter() -> Self::Filter {
        EntityDataFilter::new()
    }

    fn validate() -> bool {
        true
    }
}

impl<T: EntityData> ViewElement for Read<T> {
    type Component = T;
}

#[derive(Debug)]
pub struct Write<T: EntityData>(PhantomData<T>);

impl<'a, T: EntityData> View<'a> for Write<T> {
    type Iter = BorrowedIter<'a, IterMut<'a, T>>;
    type Filter = EntityDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        chunk.entity_data_mut().unwrap().into_iter()
    }

    fn filter() -> Self::Filter {
        EntityDataFilter::new()
    }

    fn validate() -> bool {
        true
    }
}

impl<T: EntityData> ViewElement for Write<T> {
    type Component = T;
}

#[derive(Debug)]
pub struct Shared<T: SharedData>(PhantomData<T>);

impl<T: SharedData> ReadOnly for Shared<T> {}

impl<'a, T: SharedData> View<'a> for Shared<T> {
    type Iter = Take<Repeat<&'a T>>;
    type Filter = SharedDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        unsafe {
            let data: &T = chunk.shared_component().unwrap();
            std::iter::repeat(data).take(chunk.len())
        }
    }

    fn filter() -> Self::Filter {
        SharedDataFilter::new()
    }

    fn validate() -> bool {
        true
    }
}

impl<T: SharedData> ViewElement for Shared<T> {
    type Component = Shared<T>;
}

macro_rules! impl_view_tuple {
    ( $( $ty: ident ),* ) => {
        impl<'a, $( $ty: ViewElement + View<'a> ),* > View<'a> for ($( $ty, )*) {
            type Iter = itertools::Zip<($( $ty::Iter, )*)>;
            type Filter = And<($( $ty::Filter, )*)>;

            fn fetch(chunk: &'a Chunk) -> Self::Iter {
                multizip(($( $ty::fetch(chunk), )*))
            }

            fn filter() -> Self::Filter {
                And {
                    filters: ($( $ty::filter(), )*)
                }
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
        }

        impl<$( $ty: ReadOnly ),*> ReadOnly for ($( $ty, )*) {}
    };
}

impl_view_tuple!(A);
impl_view_tuple!(A, B);
impl_view_tuple!(A, B, C);
impl_view_tuple!(A, B, C, D);
impl_view_tuple!(A, B, C, D, E);

pub trait ArchetypeFilter {
    fn filter(&self, archetype: &Archetype) -> bool;
}

pub trait ChunkFilter {
    fn filter(&self, chunk: &Chunk) -> bool;
}

#[derive(Debug)]
pub struct Passthrough;

impl ArchetypeFilter for Passthrough {
    #[inline]
    fn filter(&self, _: &Archetype) -> bool {
        true
    }
}

impl ChunkFilter for Passthrough {
    #[inline]
    fn filter(&self, _: &Chunk) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct Not<F> {
    filter: F,
}

impl<F: ArchetypeFilter> ArchetypeFilter for Not<F> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        !self.filter.filter(archetype)
    }
}

impl<F: ChunkFilter> ChunkFilter for Not<F> {
    #[inline]
    fn filter(&self, chunk: &Chunk) -> bool {
        !self.filter.filter(chunk)
    }
}

#[derive(Debug)]
pub struct And<T> {
    filters: T,
}

macro_rules! impl_and_filter {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: ArchetypeFilter ),*> ArchetypeFilter for And<($( $ty, )*)> {
            #[inline]
            fn filter(&self, archetype: &Archetype) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter(archetype) )&&*
            }
        }

        impl<$( $ty: ChunkFilter ),*> ChunkFilter for And<($( $ty, )*)> {
            #[inline]
            fn filter(&self, chunk: &Chunk) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter(chunk) )&&*
            }
        }
    };
}

impl_and_filter!(A);
impl_and_filter!(A, B);
impl_and_filter!(A, B, C);
impl_and_filter!(A, B, C, D);
impl_and_filter!(A, B, C, D, E);
impl_and_filter!(A, B, C, D, E, F);

#[derive(Debug)]
pub struct EntityDataFilter<T>(PhantomData<T>);

impl<T: EntityData> EntityDataFilter<T> {
    fn new() -> Self {
        EntityDataFilter(PhantomData)
    }
}

impl<T: EntityData> ArchetypeFilter for EntityDataFilter<T> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        archetype.has_component::<T>()
    }
}

#[derive(Debug)]
pub struct SharedDataFilter<T>(PhantomData<T>);

impl<T: SharedData> SharedDataFilter<T> {
    fn new() -> Self {
        SharedDataFilter(PhantomData)
    }
}

impl<T: SharedData> ArchetypeFilter for SharedDataFilter<T> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        archetype.has_shared::<T>()
    }
}

#[derive(Debug)]
pub struct SharedDataValueFilter<'a, T> {
    value: &'a T,
}

impl<'a, T: SharedData> SharedDataValueFilter<'a, T> {
    fn new(value: &'a T) -> Self {
        SharedDataValueFilter { value }
    }
}

impl<'a, T: SharedData> ChunkFilter for SharedDataValueFilter<'a, T> {
    #[inline]
    fn filter(&self, chunk: &Chunk) -> bool {
        unsafe { chunk.shared_component::<T>() }.map_or(false, |s| s == self.value)
    }
}

#[derive(Debug)]
pub struct Query<'a, V: View<'a>, A: ArchetypeFilter, C: ChunkFilter> {
    world: &'a World,
    view: PhantomData<V>,
    arch_filter: A,
    chunk_filter: C,
}

impl<'a, V: View<'a>, A: ArchetypeFilter, C: ChunkFilter> Query<'a, V, A, C>
where
    A: 'a,
    C: 'a,
{
    pub fn with_entity_data<T: EntityData>(self) -> Query<'a, V, And<(A, EntityDataFilter<T>)>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And {
                filters: (self.arch_filter, EntityDataFilter::new()),
            },
            chunk_filter: self.chunk_filter,
        }
    }

    pub fn without_entity_data<T: EntityData>(
        self,
    ) -> Query<'a, V, And<(A, Not<EntityDataFilter<T>>)>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And {
                filters: (
                    self.arch_filter,
                    Not {
                        filter: EntityDataFilter::new(),
                    },
                ),
            },
            chunk_filter: self.chunk_filter,
        }
    }

    pub fn with_shared_data<T: SharedData>(self) -> Query<'a, V, And<(A, SharedDataFilter<T>)>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And {
                filters: (self.arch_filter, SharedDataFilter::new()),
            },
            chunk_filter: self.chunk_filter,
        }
    }

    pub fn without_shared_data<T: SharedData>(
        self,
    ) -> Query<'a, V, And<(A, Not<SharedDataFilter<T>>)>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And {
                filters: (
                    self.arch_filter,
                    Not {
                        filter: SharedDataFilter::new(),
                    },
                ),
            },
            chunk_filter: self.chunk_filter,
        }
    }

    pub fn with_shared_data_value<'b, T: SharedData>(
        self,
        value: &'b T,
    ) -> Query<'a, V, A, And<(C, SharedDataValueFilter<'b, T>)>> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: self.arch_filter,
            chunk_filter: And {
                filters: (self.chunk_filter, SharedDataValueFilter::new(value)),
            },
        }
    }

    pub fn without_shared_data_value<'b, T: SharedData>(
        self,
        value: &'b T,
    ) -> Query<'a, V, A, And<(C, Not<SharedDataValueFilter<'b, T>>)>> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: self.arch_filter,
            chunk_filter: And {
                filters: (
                    self.chunk_filter,
                    Not {
                        filter: SharedDataValueFilter::new(value),
                    },
                ),
            },
        }
    }

    pub fn into_chunks(self) -> impl Iterator<Item = ChunkView<'a, V>> {
        let world = self.world;
        let arch = self.arch_filter;
        let chunk = self.chunk_filter;
        world
            .archetypes
            .iter()
            .filter(move |a| arch.filter(a))
            .flat_map(|a| a.chunks())
            .filter(move |c| chunk.filter(c))
            .map(|c| ChunkView {
                chunk: c,
                view: PhantomData,
            })
    }

    pub fn into_data(self) -> impl Iterator<Item = <<V as View<'a>>::Iter as Iterator>::Item> {
        self.into_chunks().flat_map(|mut c| c.iter())
    }

    pub fn into_data_with_entities(
        self,
    ) -> impl Iterator<Item = (Entity, <<V as View<'a>>::Iter as Iterator>::Item)> {
        self.into_chunks().flat_map(|mut c| c.iter_with_entities())
    }
}

#[derive(Debug)]
pub struct ChunkView<'a, V: View<'a>> {
    chunk: &'a Chunk,
    view: PhantomData<V>,
}

impl<'a, V: View<'a>> ChunkView<'a, V> {
    pub fn entities(&self) -> &[Entity] {
        unsafe { self.chunk.entities() }
    }

    pub fn iter(&mut self) -> V::Iter {
        V::fetch(self.chunk)
    }

    pub fn iter_with_entities(
        &mut self,
    ) -> impl Iterator<Item = (Entity, <<V as View<'a>>::Iter as Iterator>::Item)> + 'a {
        unsafe {
            self.chunk
                .entities()
                .iter()
                .map(|e| *e)
                .zip(V::fetch(self.chunk))
        }
    }

    pub fn shared_data<T: SharedData>(&self) -> Option<&T> {
        unsafe { self.chunk.shared_component() }
    }

    pub fn data<T: EntityData>(&self) -> Option<BorrowedSlice<'a, T>> {
        self.chunk.entity_data()
    }

    pub fn data_mut<T: EntityData>(&self) -> Option<BorrowedMutSlice<'a, T>> {
        self.chunk.entity_data_mut()
    }
}
