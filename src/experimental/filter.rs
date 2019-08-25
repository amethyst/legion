use crate::experimental::borrow::Borrow;
use crate::experimental::borrow::Ref;
use crate::experimental::storage::ArchetypeData;
use crate::experimental::storage::ArchetypeId;
use crate::experimental::storage::ChunkId;
use crate::experimental::storage::Component;
use crate::experimental::storage::ComponentStorage;
use crate::experimental::storage::ComponentTypeId;
use crate::experimental::storage::ComponentTypes;
use crate::experimental::storage::SliceVecIter;
use crate::experimental::storage::Storage;
use crate::experimental::storage::Tag;
use crate::experimental::storage::TagTypeId;
use crate::experimental::storage::TagTypes;
use crate::experimental::world::World;
use std::collections::HashMap;
use std::iter::Enumerate;
use std::iter::Repeat;
use std::iter::Take;
use std::marker::PhantomData;
use std::ops::Deref;
use std::slice::Iter;

pub mod filter_fns {
    ///! Contains functions for constructing filters.
    use super::*;

    /// Creates an entity data filter which includes chunks that contain
    /// entity data components of type `T`.
    pub fn component<T: Component>() -> EntityFilterTuple<ComponentFilter<T>, Passthrough> {
        EntityFilterTuple::new(ComponentFilter::new(), Passthrough)
    }

    /// Creates a shared data filter which includes chunks that contain
    /// shared data components of type `T`.
    pub fn tag<T: Tag>() -> EntityFilterTuple<TagFilter<T>, Passthrough> {
        EntityFilterTuple::new(TagFilter::new(), Passthrough)
    }

    /// Creates a shared data filter which includes chunks that contain
    /// specific shared data values.
    pub fn tag_value<'a, T: Tag>(
        data: &'a T,
    ) -> EntityFilterTuple<TagFilter<T>, TagValueFilter<'a, T>> {
        EntityFilterTuple::new(TagFilter::new(), TagValueFilter::new(data))
    }

    /// Creates a filter which includes chunks for which entity data components
    /// of type `T` have changed since the filter was last executed.
    pub fn changed<T: Component>(
    ) -> EntityFilterTuple<ComponentFilter<T>, ComponentChangedFilter<T>> {
        EntityFilterTuple::new(ComponentFilter::new(), ComponentChangedFilter::new())
    }
}

pub trait FilterResult {
    fn coalesce_and(self, other: Self) -> Self;
    fn coalesce_or(self, other: Self) -> Self;
    fn is_pass(&self) -> bool;
}

impl FilterResult for Option<bool> {
    #[inline]
    fn coalesce_and(self, other: Self) -> Self {
        match self {
            Some(x) => other.map(|y| x && y).or(Some(x)),
            None => other,
        }
    }

    #[inline]
    fn coalesce_or(self, other: Self) -> Self {
        match self {
            Some(x) => other.map(|y| x || y).or(Some(x)),
            None => other,
        }
    }

    #[inline]
    fn is_pass(&self) -> bool {
        self.unwrap_or(true)
    }
}

pub trait Filter<'a, T: Copy> {
    type Iter: Iterator;

    fn collect(&self, source: T) -> Self::Iter;
    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool>;
}

#[derive(Copy, Clone)]
pub struct ArchetypeFilterData<'a> {
    pub component_types: &'a ComponentTypes,
    pub tag_types: &'a TagTypes,
}

#[derive(Copy, Clone)]
pub struct ChunkFilterData<'a> {
    pub archetype_data: &'a ArchetypeData,
}

pub trait ActiveFilter {}

pub trait EntityFilter {
    type ArchetypeFilter: for<'a> Filter<'a, ArchetypeFilterData<'a>>;
    type ChunkFilter: for<'a> Filter<'a, ChunkFilterData<'a>>;

    fn filters(&mut self) -> (&mut Self::ArchetypeFilter, &mut Self::ChunkFilter);

    fn iter_archetype_indexes<'a, 'b>(
        &'a mut self,
        storage: &'b Storage,
    ) -> FilterArchIter<'b, 'a, Self::ArchetypeFilter>;

    fn iter_chunk_indexes<'a, 'b>(
        &'a mut self,
        archetype: &'b ArchetypeData,
    ) -> FilterChunkIter<'b, 'a, Self::ChunkFilter>;

    fn iter<'a, 'b>(
        &'a mut self,
        storage: &'b Storage,
    ) -> FilterEntityIter<'b, 'a, Self::ArchetypeFilter, Self::ChunkFilter>;
}

#[derive(Debug)]
pub struct EntityFilterTuple<A, C> {
    pub arch_filter: A,
    pub chunk_filter: C,
}

impl<A, C> EntityFilterTuple<A, C>
where
    A: for<'a> Filter<'a, ArchetypeFilterData<'a>>,
    C: for<'a> Filter<'a, ChunkFilterData<'a>>,
{
    pub fn new(arch_filter: A, chunk_filter: C) -> Self {
        Self {
            arch_filter,
            chunk_filter,
        }
    }
}

impl<A, C> EntityFilter for EntityFilterTuple<A, C>
where
    A: for<'a> Filter<'a, ArchetypeFilterData<'a>>,
    C: for<'a> Filter<'a, ChunkFilterData<'a>>,
{
    type ArchetypeFilter = A;
    type ChunkFilter = C;

    fn filters(&mut self) -> (&mut Self::ArchetypeFilter, &mut Self::ChunkFilter) {
        (&mut self.arch_filter, &mut self.chunk_filter)
    }

    fn iter_archetype_indexes<'a, 'b>(
        &'a mut self,
        storage: &'b Storage,
    ) -> FilterArchIter<'b, 'a, A> {
        let data = ArchetypeFilterData {
            component_types: storage.component_types(),
            tag_types: storage.tag_types(),
        };

        let iter = self.arch_filter.collect(data);
        FilterArchIter {
            archetypes: iter.enumerate(),
            filter: &mut self.arch_filter,
        }
    }

    fn iter_chunk_indexes<'a, 'b>(
        &'a mut self,
        archetype: &'b ArchetypeData,
    ) -> FilterChunkIter<'b, 'a, C> {
        let data = ChunkFilterData {
            archetype_data: archetype,
        };

        let iter = self.chunk_filter.collect(data);
        FilterChunkIter {
            chunks: iter.enumerate(),
            filter: &mut self.chunk_filter,
        }
    }

    fn iter<'a, 'b>(&'a mut self, storage: &'b Storage) -> FilterEntityIter<'b, 'a, A, C> {
        let data = ArchetypeFilterData {
            component_types: storage.component_types(),
            tag_types: storage.tag_types(),
        };

        let iter = self.arch_filter.collect(data).enumerate();
        FilterEntityIter {
            storage: storage,
            arch_filter: &mut self.arch_filter,
            chunk_filter: &mut self.chunk_filter,
            archetypes: iter,
            chunks: None,
        }
    }
}

impl<A, C> std::ops::Not for EntityFilterTuple<A, C>
where
    A: std::ops::Not,
    C: std::ops::Not,
{
    type Output = EntityFilterTuple<A::Output, C::Output>;

    #[inline]
    fn not(self) -> Self::Output {
        EntityFilterTuple {
            arch_filter: !self.arch_filter,
            chunk_filter: !self.chunk_filter,
        }
    }
}

impl<'a, A1, C1, A2, C2> std::ops::BitAnd<EntityFilterTuple<A2, C2>> for EntityFilterTuple<A1, C1>
where
    A1: std::ops::BitAnd<A2>,
    C1: std::ops::BitAnd<C2>,
{
    type Output = EntityFilterTuple<A1::Output, C1::Output>;

    #[inline]
    fn bitand(self, rhs: EntityFilterTuple<A2, C2>) -> Self::Output {
        EntityFilterTuple {
            arch_filter: self.arch_filter & rhs.arch_filter,
            chunk_filter: self.chunk_filter & rhs.chunk_filter,
        }
    }
}

impl<'a, A1, C1, A2, C2> std::ops::BitOr<EntityFilterTuple<A2, C2>> for EntityFilterTuple<A1, C1>
where
    A1: std::ops::BitOr<A2>,
    C1: std::ops::BitOr<C2>,
{
    type Output = EntityFilterTuple<A1::Output, C1::Output>;

    #[inline]
    fn bitor(self, rhs: EntityFilterTuple<A2, C2>) -> Self::Output {
        EntityFilterTuple {
            arch_filter: self.arch_filter | rhs.arch_filter,
            chunk_filter: self.chunk_filter | rhs.chunk_filter,
        }
    }
}

pub struct FilterArchIter<'a, 'b, F: Filter<'a, ArchetypeFilterData<'a>>> {
    filter: &'b mut F,
    archetypes: Enumerate<F::Iter>,
}

impl<'a, 'b, F: Filter<'a, ArchetypeFilterData<'a>>> Iterator for FilterArchIter<'a, 'b, F> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((i, data)) = self.archetypes.next() {
            if self.filter.is_match(&data).is_pass() {
                return Some(i);
            }
        }

        None
    }
}

pub struct FilterChunkIter<'a, 'b, F: Filter<'a, ChunkFilterData<'a>>> {
    filter: &'b mut F,
    chunks: Enumerate<F::Iter>,
}

impl<'a, 'b, F: Filter<'a, ChunkFilterData<'a>>> Iterator for FilterChunkIter<'a, 'b, F> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((i, data)) = self.chunks.next() {
            if self.filter.is_match(&data).is_pass() {
                return Some(i);
            }
        }

        None
    }
}

pub struct FilterEntityIter<
    'a,
    'b,
    Arch: Filter<'a, ArchetypeFilterData<'a>>,
    Chunk: Filter<'a, ChunkFilterData<'a>>,
> {
    storage: &'a Storage,
    arch_filter: &'b mut Arch,
    chunk_filter: &'b mut Chunk,
    archetypes: Enumerate<Arch::Iter>,
    chunks: Option<(ArchetypeId, Borrow<'a>, Enumerate<Chunk::Iter>)>,
}

impl<'a, 'b, Arch: Filter<'a, ArchetypeFilterData<'a>>, Chunk: Filter<'a, ChunkFilterData<'a>>>
    Iterator for FilterEntityIter<'a, 'b, Arch, Chunk>
{
    type Item = ChunkId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((arch_id, _, ref mut chunks)) = self.chunks {
                while let Some((chunk_index, chunk_data)) = chunks.next() {
                    if self.chunk_filter.is_match(&chunk_data).is_pass() {
                        return Some(ChunkId::new(arch_id, chunk_index));
                    }
                }
            }
            loop {
                match self.archetypes.next() {
                    Some((arch_index, arch_data)) => {
                        if self.arch_filter.is_match(&arch_data).is_pass() {
                            self.chunks = {
                                let (borrow, archetype) = unsafe {
                                    self.storage
                                        .data(arch_index)
                                        .unwrap()
                                        .deref()
                                        .get()
                                        .deconstruct()
                                };
                                let data = ChunkFilterData {
                                    archetype_data: archetype,
                                };

                                Some((
                                    ArchetypeId::new(arch_index),
                                    borrow,
                                    self.chunk_filter.collect(data).enumerate(),
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

#[derive(Debug)]
pub struct Passthrough;

impl<'a> Filter<'a, ArchetypeFilterData<'a>> for Passthrough {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: ArchetypeFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.component_types.len())
    }

    fn is_match(&mut self, _: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl<'a> Filter<'a, ChunkFilterData<'a>> for Passthrough {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: ChunkFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.archetype_data.len())
    }

    fn is_match(&mut self, _: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl std::ops::Not for Passthrough {
    type Output = Passthrough;

    #[inline]
    fn not(self) -> Self::Output {
        self
    }
}

impl<'a, Rhs> std::ops::BitAnd<Rhs> for Passthrough {
    type Output = Rhs;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        rhs
    }
}

impl<'a, Rhs> std::ops::BitOr<Rhs> for Passthrough {
    type Output = Rhs;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        rhs
    }
}

#[derive(Debug)]
pub struct Not<F> {
    pub filter: F,
}

impl<F> ActiveFilter for Not<F> {}

impl<'a, T: Copy, F: Filter<'a, T>> Filter<'a, T> for Not<F> {
    type Iter = F::Iter;

    fn collect(&self, source: T) -> Self::Iter {
        self.filter.collect(source)
    }

    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        self.filter.is_match(item).map(|x| !x)
    }
}

impl<'a, F, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for Not<F> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, F> std::ops::BitAnd<Passthrough> for Not<F> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, F, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for Not<F> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, F> std::ops::BitOr<Passthrough> for Not<F> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}

#[derive(Debug)]
pub struct And<T> {
    pub filters: T,
}

macro_rules! impl_and_filter {
    ( $( $ty: ident => $ty2: ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for And<($( $ty, )*)> {}

        impl<'a, T: Copy, $( $ty: Filter<'a, T> ),*> Filter<'a, T> for And<($( $ty, )*)> {
            type Iter = itertools::Zip<( $( $ty::Iter ),* )>;

            fn collect(&self, source: T) -> Self::Iter {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let iters = (
                    $( $ty.collect(source) ),*
                );
                itertools::multizip(iters)
            }

            fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let ($( $ty2, )*) = item;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_and($ty.is_match($ty2)); )*
                result
            }
        }

        impl<$( $ty ),*> std::ops::Not for And<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for And<($( $ty, )*)> {
            type Output = And<($( $ty, )* Rhs)>;

            #[inline]
            fn bitand(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                And {
                    filters: ($( $ty, )* rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitAnd<Passthrough> for And<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitand(self, _: Passthrough) -> Self::Output {
                self
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for And<($( $ty, )*)> {
            type Output = Or<(Self, Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                Or {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitOr<Passthrough> for And<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitor(self, _: Passthrough) -> Self::Output {
                self
            }
        }
    }
}

impl_and_filter!(A => a, B => b);
impl_and_filter!(A => a, B => b, C => c);
impl_and_filter!(A => a, B => b, C => c, D => d);
impl_and_filter!(A => a, B => b, C => c, D => d, E => e);
impl_and_filter!(A => a, B => b, C => c, D => d, E => e, F => f);

#[derive(Debug)]
pub struct Or<T> {
    pub filters: T,
}

macro_rules! impl_or_filter {
    ( $( $ty: ident => $ty2: ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for Or<($( $ty, )*)> {}

        impl<'a, T: Copy, $( $ty: Filter<'a, T> ),*> Filter<'a, T> for Or<($( $ty, )*)> {
            type Iter = itertools::Zip<( $( $ty::Iter ),* )>;

            fn collect(&self, source: T) -> Self::Iter {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let iters = (
                    $( $ty.collect(source) ),*
                );
                itertools::multizip(iters)
            }

            fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let ($( $ty2, )*) = item;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_or($ty.is_match($ty2)); )*
                result
            }
        }

        impl<$( $ty ),*> std::ops::Not for Or<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for Or<($( $ty, )*)> {
            type Output = And<(Self, Rhs)>;

            #[inline]
            fn bitand(self, rhs: Rhs) -> Self::Output {
                And {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitAnd<Passthrough> for Or<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitand(self, _: Passthrough) -> Self::Output {
                self
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for Or<($( $ty, )*)> {
            type Output = Or<($( $ty, )* Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                Or {
                    filters: ($( $ty, )* rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitOr<Passthrough> for Or<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitor(self, _: Passthrough) -> Self::Output {
                self
            }
        }
    }
}

impl_or_filter!(A => a, B => b);
impl_or_filter!(A => a, B => b, C => c);
impl_or_filter!(A => a, B => b, C => c, D => d);
impl_or_filter!(A => a, B => b, C => c, D => d, E => e);
impl_or_filter!(A => a, B => b, C => c, D => d, E => e, F => f);

#[derive(Debug)]
pub struct ComponentFilter<T>(PhantomData<T>);

impl<T: Component> ComponentFilter<T> {
    fn new() -> Self {
        ComponentFilter(PhantomData)
    }
}

impl<T> ActiveFilter for ComponentFilter<T> {}

impl<'a, T: Component> Filter<'a, ArchetypeFilterData<'a>> for ComponentFilter<T> {
    type Iter = SliceVecIter<'a, ComponentTypeId>;

    fn collect(&self, source: ArchetypeFilterData<'a>) -> Self::Iter {
        source.component_types.iter()
    }

    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&ComponentTypeId::of::<T>()))
    }
}

impl<T> std::ops::Not for ComponentFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for ComponentFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitAnd<Passthrough> for ComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for ComponentFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitOr<Passthrough> for ComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}

#[derive(Debug)]
pub struct TagFilter<T>(PhantomData<T>);

impl<T: Tag> TagFilter<T> {
    fn new() -> Self {
        TagFilter(PhantomData)
    }
}

impl<T> ActiveFilter for TagFilter<T> {}

impl<'a, T: Tag> Filter<'a, ArchetypeFilterData<'a>> for TagFilter<T> {
    type Iter = SliceVecIter<'a, TagTypeId>;

    fn collect(&self, source: ArchetypeFilterData<'a>) -> Self::Iter {
        source.tag_types.iter()
    }

    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&TagTypeId::of::<T>()))
    }
}

impl<T> std::ops::Not for TagFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for TagFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitAnd<Passthrough> for TagFilter<T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for TagFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitOr<Passthrough> for TagFilter<T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}

#[derive(Debug)]
pub struct TagValueFilter<'a, T> {
    value: &'a T,
}

impl<'a, T: Tag> TagValueFilter<'a, T> {
    fn new(value: &'a T) -> Self {
        TagValueFilter { value }
    }
}

impl<'a, T> ActiveFilter for TagValueFilter<'a, T> {}

impl<'a, 'b, T: Tag> Filter<'a, ChunkFilterData<'a>> for TagValueFilter<'b, T> {
    type Iter = Iter<'a, T>;

    fn collect(&self, source: ChunkFilterData<'a>) -> Self::Iter {
        unsafe {
            source
                .archetype_data
                .tags(&TagTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .iter()
        }
    }

    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(**item == *self.value)
    }
}

impl<'a, T> std::ops::Not for TagValueFilter<'a, T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for TagValueFilter<'a, T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitAnd<Passthrough> for TagValueFilter<'a, T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, T, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for TagValueFilter<'a, T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T> std::ops::BitOr<Passthrough> for TagValueFilter<'a, T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}

#[derive(Debug)]
pub struct ComponentChangedFilter<T: Component> {
    versions: HashMap<ChunkId, usize>,
    phantom: PhantomData<T>,
}

impl<T: Component> ComponentChangedFilter<T> {
    fn new() -> ComponentChangedFilter<T> {
        ComponentChangedFilter {
            versions: HashMap::new(),
            phantom: PhantomData,
        }
    }
}

impl<T: Component> ActiveFilter for ComponentChangedFilter<T> {}

impl<'a, T: Component> Filter<'a, ChunkFilterData<'a>> for ComponentChangedFilter<T> {
    type Iter = Iter<'a, ComponentStorage>;

    fn collect(&self, source: ChunkFilterData<'a>) -> Self::Iter {
        source.archetype_data.iter_component_chunks()
    }

    fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
        use std::collections::hash_map::Entry;
        let components = item.components(&ComponentTypeId::of::<T>()).unwrap();
        let version = components.version();
        match self.versions.entry(item.id()) {
            Entry::Occupied(mut entry) => Some(entry.insert(version) != version),
            Entry::Vacant(entry) => {
                entry.insert(version);
                Some(true)
            }
        }
    }
}

impl<'a, T: Component> std::ops::Not for ComponentChangedFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for ComponentChangedFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitAnd<Passthrough> for ComponentChangedFilter<T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for ComponentChangedFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitOr<Passthrough> for ComponentChangedFilter<T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}

#[cfg(test)]
mod test {
    use super::filter_fns::*;
    use super::*;

    #[test]
    pub fn create() {
        let filter = component::<usize>() | tag_value(&5isize);
        println!("{:?}", filter);
    }
}
