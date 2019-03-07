use itertools::multizip;
use std::iter::Repeat;
use std::iter::Take;
use std::marker::PhantomData;
use std::slice::Iter;
use std::slice::IterMut;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

use crate::*;

pub trait View<'a>: Sized + Send + Sync + 'static {
    type Iter: Iterator + 'a;
    type Filter: Filter;

    fn fetch(chunk: &'a Chunk) -> Self::Iter;
    fn filter() -> Self::Filter;
    fn validate() -> bool;
    fn reads<T: EntityData>() -> bool;
    fn writes<T: EntityData>() -> bool;
}

pub trait ViewElement {
    type Component;
}

pub trait IntoQuery<'a>: View<'a> {
    fn query() -> QueryDef<'a, Self, <Self as View<'a>>::Filter>;
}

impl<'a, T: View<'a>> IntoQuery<'a> for T {
    fn query() -> QueryDef<'a, Self, Self::Filter> {
        if !Self::validate() {
            panic!("invalid view, please ensure the view contains no duplicate component types");
        }

        QueryDef {
            view: PhantomData,
            filter: Self::filter(),
        }
    }
}

#[derive(Debug)]
pub struct Read<T: EntityData>(PhantomData<T>);

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

    fn reads<D: EntityData>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    fn writes<D: EntityData>() -> bool {
        false
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

    fn reads<D: EntityData>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }

    fn writes<D: EntityData>() -> bool {
        TypeId::of::<T>() == TypeId::of::<D>()
    }
}

impl<T: EntityData> ViewElement for Write<T> {
    type Component = T;
}

#[derive(Debug)]
pub struct Shared<T: SharedData>(PhantomData<T>);

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

    fn reads<D: EntityData>() -> bool {
        false
    }

    fn writes<D: EntityData>() -> bool {
        false
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

            fn reads<Data: EntityData>() -> bool {
                $( $ty::reads::<Data>() )||*
            }

            fn writes<Data: EntityData>() -> bool {
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

pub trait Filter: Sync + Sized {
    fn filter_archetype(&self, archetype: &Archetype) -> bool;
    fn filter_chunk(&self, chunk: &Chunk) -> bool;

    fn or<T: Filter>(self, filter: T) -> Or<(Self, T)> {
        Or {
            filters: (self, filter),
        }
    }

    fn and<T: Filter>(self, filter: T) -> And<(Self, T)> {
        And {
            filters: (self, filter),
        }
    }
}

pub mod filter {
    use super::*;

    pub fn entity_data<T: EntityData>() -> EntityDataFilter<T> {
        EntityDataFilter::new()
    }

    pub fn shared_data<T: SharedData>() -> SharedDataFilter<T> {
        SharedDataFilter::new()
    }

    pub fn shared_data_filter<'a, T: SharedData>(data: &'a T) -> SharedDataValueFilter<'a, T> {
        SharedDataValueFilter::new(data)
    }

    pub fn changed<T: EntityData>() -> EntityDataChangedFilter<T> {
        EntityDataChangedFilter::new()
    }
}

#[derive(Debug)]
pub struct Passthrough;

impl Filter for Passthrough {
    #[inline]
    fn filter_archetype(&self, _: &Archetype) -> bool {
        true
    }

    #[inline]
    fn filter_chunk(&self, _: &Chunk) -> bool {
        true
    }
}

impl<Rhs: Filter> std::ops::BitAnd<Rhs> for Passthrough {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<Rhs: Filter> std::ops::BitOr<Rhs> for Passthrough {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

#[derive(Debug)]
pub struct Not<F> {
    filter: F,
}

impl<F: Filter> Filter for Not<F> {
    #[inline]
    fn filter_archetype(&self, archetype: &Archetype) -> bool {
        !self.filter.filter_archetype(archetype)
    }

    #[inline]
    fn filter_chunk(&self, chunk: &Chunk) -> bool {
        !self.filter.filter_chunk(chunk)
    }
}

impl<F: Filter, Rhs: Filter> std::ops::BitAnd<Rhs> for Not<F> {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<F: Filter, Rhs: Filter> std::ops::BitOr<Rhs> for Not<F> {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

#[derive(Debug)]
pub struct And<T> {
    filters: T,
}

macro_rules! impl_and_filter {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Filter ),*> Filter for And<($( $ty, )*)> {
            #[inline]
            fn filter_archetype(&self, archetype: &Archetype) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter_archetype(archetype) )&&*
            }

            #[inline]
            fn filter_chunk(&self, chunk: &Chunk) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter_chunk(chunk) )&&*
            }
        }

        impl<$( $ty: Filter ),*, Rhs: Filter> std::ops::BitAnd<Rhs> for And<($( $ty, )*)> {
            type Output = And<($( $ty, )* Rhs)>;

            fn bitand(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                And {
                    filters: ($( $ty, )* rhs),
                }
            }
        }

        impl<$( $ty: Filter ),*, Rhs: Filter> std::ops::BitOr<Rhs> for And<($( $ty, )*)> {
            type Output = Or<(Self, Rhs)>;

            fn bitor(self, rhs: Rhs) -> Self::Output {
                Or {
                    filters: (self, rhs),
                }
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
pub struct Or<T> {
    filters: T,
}

macro_rules! impl_or_filter {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Filter ),*> Filter for Or<($( $ty, )*)> {
            #[inline]
            fn filter_archetype(&self, archetype: &Archetype) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter_archetype(archetype) )||*
            }

            #[inline]
            fn filter_chunk(&self, chunk: &Chunk) -> bool {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                $( $ty.filter_chunk(chunk) )||*
            }
        }

        impl<$( $ty: Filter ),*, Rhs: Filter> std::ops::BitAnd<Rhs> for Or<($( $ty, )*)> {
            type Output = And<(Self, Rhs)>;

            fn bitand(self, rhs: Rhs) -> Self::Output {
                And {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty: Filter ),*, Rhs: Filter> std::ops::BitOr<Rhs> for Or<($( $ty, )*)> {
            type Output = Or<($( $ty, )* Rhs)>;

            fn bitor(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                Or {
                    filters: ($( $ty, )* rhs),
                }
            }
        }
    };
}

impl_or_filter!(A);
impl_or_filter!(A, B);
impl_or_filter!(A, B, C);
impl_or_filter!(A, B, C, D);
impl_or_filter!(A, B, C, D, E);
impl_or_filter!(A, B, C, D, E, F);

#[derive(Debug)]
pub struct EntityDataFilter<T>(PhantomData<T>);

impl<T: EntityData> EntityDataFilter<T> {
    fn new() -> Self {
        EntityDataFilter(PhantomData)
    }
}

impl<T: EntityData> Filter for EntityDataFilter<T> {
    #[inline]
    fn filter_archetype(&self, archetype: &Archetype) -> bool {
        archetype.has_component::<T>()
    }

    #[inline]
    fn filter_chunk(&self, _: &Chunk) -> bool {
        true
    }
}

impl<Rhs: Filter, T: EntityData> std::ops::BitAnd<Rhs> for EntityDataFilter<T> {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<Rhs: Filter, T: EntityData> std::ops::BitOr<Rhs> for EntityDataFilter<T> {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

#[derive(Debug)]
pub struct SharedDataFilter<T>(PhantomData<T>);

impl<T: SharedData> SharedDataFilter<T> {
    fn new() -> Self {
        SharedDataFilter(PhantomData)
    }
}

impl<T: SharedData> Filter for SharedDataFilter<T> {
    #[inline]
    fn filter_archetype(&self, archetype: &Archetype) -> bool {
        archetype.has_shared::<T>()
    }

    #[inline]
    fn filter_chunk(&self, _: &Chunk) -> bool {
        true
    }
}

impl<Rhs: Filter, T: SharedData> std::ops::BitAnd<Rhs> for SharedDataFilter<T> {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<Rhs: Filter, T: SharedData> std::ops::BitOr<Rhs> for SharedDataFilter<T> {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
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

impl<'a, T: SharedData> Filter for SharedDataValueFilter<'a, T> {
    #[inline]
    fn filter_archetype(&self, _: &Archetype) -> bool {
        true
    }

    #[inline]
    fn filter_chunk(&self, chunk: &Chunk) -> bool {
        unsafe { chunk.shared_component::<T>() }.map_or(false, |s| s == self.value)
    }
}

impl<'a, Rhs: Filter, T: SharedData> std::ops::BitAnd<Rhs> for SharedDataValueFilter<'a, T> {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, Rhs: Filter, T: SharedData> std::ops::BitOr<Rhs> for SharedDataValueFilter<'a, T> {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct EntityDataChangedFilter<T: EntityData> {
    versions: Mutex<HashMap<ChunkId, usize>>,
    phantom: PhantomData<T>,
}

impl<T: EntityData> EntityDataChangedFilter<T> {
    fn new() -> EntityDataChangedFilter<T> {
        EntityDataChangedFilter {
            versions: Mutex::new(HashMap::new()),
            phantom: PhantomData,
        }
    }
}

impl<T: EntityData> Filter for EntityDataChangedFilter<T> {
    #[inline]
    fn filter_archetype(&self, _: &Archetype) -> bool {
        true
    }

    fn filter_chunk(&self, chunk: &Chunk) -> bool {
        use std::collections::hash_map::Entry;
        if let Some(version) = chunk.entity_data_version::<T>() {
            let mut versions = self.versions.lock();
            match versions.entry(chunk.id()) {
                Entry::Occupied(mut entry) => {
                    let existing = entry.get_mut();
                    let changed = *existing != version;
                    *existing = version;
                    changed
                }
                Entry::Vacant(entry) => {
                    entry.insert(version);
                    true
                }
            }
        } else {
            false
        }
    }
}

impl<Rhs: Filter, T: EntityData> std::ops::BitAnd<Rhs> for EntityDataChangedFilter<T> {
    type Output = And<(Self, Rhs)>;

    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<Rhs: Filter, T: EntityData> std::ops::BitOr<Rhs> for EntityDataChangedFilter<T> {
    type Output = Or<(Self, Rhs)>;

    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct ChunkViewIter<'data, 'filter, V: View<'data>, F: Filter> {
    archetypes: Iter<'data, Archetype>,
    filter: &'filter F,
    frontier: Option<Iter<'data, Chunk>>,
    view: PhantomData<V>,
}

impl<'filter, 'data, F, V> Iterator for ChunkViewIter<'data, 'filter, V, F>
where
    F: Filter,
    V: View<'data>,
{
    type Item = ChunkView<'data, V>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut inner) = self.frontier {
                for x in &mut inner.next() {
                    if self.filter.filter_chunk(x) {
                        return Some(ChunkView {
                            chunk: x,
                            view: PhantomData,
                        });
                    }
                }
            }
            loop {
                match self.archetypes.next() {
                    Some(archetype) => {
                        if self.filter.filter_archetype(archetype) {
                            self.frontier = Some(archetype.chunks().iter());
                            break;
                        }
                    }
                    None => return None,
                }
            }
        }
    }
}

pub struct ChunkDataIter<'data, 'query, V: View<'data>, F: Filter> {
    iter: ChunkViewIter<'data, 'query, V, F>,
    frontier: Option<V::Iter>,
    view: PhantomData<V>,
}

impl<'data, 'query, F: Filter, V: View<'data>> Iterator for ChunkDataIter<'data, 'query, V, F> {
    type Item = <V::Iter as Iterator>::Item;

    #[inline]
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

pub struct ChunkEntityIter<'data, 'query, V: View<'data>, F: Filter> {
    iter: ChunkViewIter<'data, 'query, V, F>,
    frontier: Option<ZipEntities<'data, V>>,
    view: PhantomData<V>,
}

impl<'data, 'query, V: View<'data>, F: Filter> Iterator for ChunkEntityIter<'data, 'query, V, F> {
    type Item = (Entity, <V::Iter as Iterator>::Item);

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

pub trait Query<'data> {
    type Filter: Filter;
    type View: View<'data>;

    fn filter<T: Filter>(self, filter: T) -> QueryDef<'data, Self::View, And<(Self::Filter, T)>>;

    fn iter_chunks<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkViewIter<'data, 'a, Self::View, Self::Filter>;

    fn iter<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkDataIter<'data, 'a, Self::View, Self::Filter>;

    fn iter_entities<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkEntityIter<'data, 'a, Self::View, Self::Filter>;

    fn for_each<'a, T>(&'a self, world: &'data World, mut f: T)
    where
        T: Fn(<<Self::View as View<'data>>::Iter as Iterator>::Item),
    {
        self.iter(world).for_each(&mut f);
    }

    #[cfg(feature = "par-iter")]
    fn par_for_each<'a, T>(&'a self, world: &'data World, f: T)
    where
        T: Fn(<<Self::View as View<'data>>::Iter as Iterator>::Item) + Send + Sync;
}

#[derive(Debug)]
pub struct QueryDef<'a, V: View<'a>, F: Filter> {
    view: PhantomData<&'a V>,
    filter: F,
}

impl<'data, V: View<'data>, F: Filter> Query<'data> for QueryDef<'data, V, F> {
    type View = V;
    type Filter = F;

    fn filter<T: Filter>(self, filter: T) -> QueryDef<'data, Self::View, And<(Self::Filter, T)>> {
        QueryDef {
            view: self.view,
            filter: self.filter.and(filter),
        }
    }

    fn iter_chunks<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkViewIter<'data, 'a, Self::View, Self::Filter> {
        ChunkViewIter {
            archetypes: world.archetypes.iter(),
            filter: &self.filter,
            frontier: None,
            view: PhantomData,
        }
    }

    fn iter<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkDataIter<'data, 'a, Self::View, Self::Filter> {
        ChunkDataIter {
            iter: self.iter_chunks(world),
            frontier: None,
            view: PhantomData,
        }
    }

    fn iter_entities<'a>(
        &'a self,
        world: &'data World,
    ) -> ChunkEntityIter<'data, 'a, Self::View, Self::Filter> {
        ChunkEntityIter {
            iter: self.iter_chunks(world),
            frontier: None,
            view: PhantomData,
        }
    }

    #[cfg(feature = "par-iter")]
    fn par_for_each<'a, T>(&'a self, world: &'data World, f: T)
    where
        T: Fn(<<V as View<'data>>::Iter as Iterator>::Item) + Send + Sync,
    {
        self.par_iter_chunks(world).for_each(|mut chunk| {
            for data in chunk.iter() {
                f(data);
            }
        });
    }
}

impl<'a, V: View<'a>, F: Filter> QueryDef<'a, V, F> {
    #[cfg(feature = "par-iter")]
    pub fn par_iter_chunks<'b>(
        &'b self,
        world: &'a World,
    ) -> impl ParallelIterator<Item = ChunkView<'a, V>> + 'b {
        let filter = &self.filter;
        let archetypes = &world.archetypes;
        archetypes
            .par_iter()
            .filter(move |a| filter.filter_archetype(a))
            .flat_map(|a| a.chunks())
            .filter(move |c| filter.filter_chunk(c))
            .map(|c| ChunkView {
                chunk: c,
                view: PhantomData,
            })
    }
}

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

#[derive(Debug)]
pub struct ChunkView<'a, V: View<'a>> {
    chunk: &'a Chunk,
    view: PhantomData<V>,
}

impl<'a, V: View<'a>> ChunkView<'a, V> {
    pub fn entities(&self) -> &'a [Entity] {
        unsafe { self.chunk.entities() }
    }

    pub fn iter(&mut self) -> V::Iter {
        V::fetch(self.chunk)
    }

    pub fn iter_entities(&mut self) -> ZipEntities<'a, V> {
        ZipEntities {
            entities: self.entities(),
            data: V::fetch(self.chunk),
            index: 0,
            view: PhantomData,
        }
    }

    pub fn shared_data<T: SharedData>(&self) -> Option<&T> {
        unsafe { self.chunk.shared_component() }
    }

    pub fn data<T: EntityData>(&self) -> Option<BorrowedSlice<'a, T>> {
        if !V::reads::<T>() {
            panic!("data type not readable via this query");
        }
        self.chunk.entity_data()
    }

    pub fn data_mut<T: EntityData>(&self) -> Option<BorrowedMutSlice<'a, T>> {
        if !V::writes::<T>() {
            panic!("data type not writable via this query");
        }
        self.chunk.entity_data_mut()
    }
}
