use crate::storage::archetype::ArchetypeTagsRef;
use crate::storage::chunk::Chunk;
use crate::storage::components::ComponentTypeId;
use crate::storage::tags::TagTypeId;
use crate::storage::{ArchetypeFilter, ChunkFilter, LayoutFilter};

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
    fn is_pass(&self) -> bool { self.unwrap_or(true) }
}

/// A marker trait for filters that are not no-ops.
pub trait ActiveFilter {}

pub trait EntityFilter: Send + Sync + Clone {
    type Layout: LayoutFilter + Send + Sync + Clone;
    type Archetype: ArchetypeFilter + Send + Sync + Clone;
    type Chunk: ChunkFilter + Send + Sync + Clone;

    fn static_filters(&self) -> (&Self::Layout, &Self::Archetype);
    fn filters(&mut self) -> (&Self::Layout, &Self::Archetype, &mut Self::Chunk);
    fn into_filters(self) -> (Self::Layout, Self::Archetype, Self::Chunk);
}

#[derive(Clone)]
pub struct EntityFilterTuple<L: LayoutFilter, A: ArchetypeFilter, C: ChunkFilter> {
    pub layout_filter: L,
    pub archetype_filter: A,
    pub chunk_filter: C,
}

impl<L: LayoutFilter, A: ArchetypeFilter, C: ChunkFilter> EntityFilterTuple<L, A, C> {
    pub fn new(layout_filter: L, archetype_filter: A, chunk_filter: C) -> Self {
        Self {
            layout_filter,
            archetype_filter,
            chunk_filter,
        }
    }
}

impl<L, A, C> LayoutFilter for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter,
    A: ArchetypeFilter,
    C: ChunkFilter,
    EntityFilterTuple<L, A, C>: EntityFilter,
{
    fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
        let (layout_filter, _) = self.static_filters();
        layout_filter.matches_layout(components, tags)
    }
}

impl<L, A, C> ArchetypeFilter for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter,
    A: ArchetypeFilter,
    C: ChunkFilter,
    EntityFilterTuple<L, A, C>: EntityFilter,
{
    fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
        let (_, arch_filter) = self.static_filters();
        arch_filter.matches_archetype(tags)
    }
}

impl<L, A, C> ChunkFilter for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter,
    A: ArchetypeFilter,
    C: ChunkFilter,
    EntityFilterTuple<L, A, C>: EntityFilter,
{
    fn prepare(&mut self) {
        let (_, _, chunk_filter) = self.filters();
        chunk_filter.prepare();
    }

    fn matches_chunk(&mut self, chunk: &Chunk) -> Option<bool> {
        let (_, _, chunk_filter) = self.filters();
        chunk_filter.matches_chunk(chunk)
    }
}

impl<L, A, C> EntityFilter for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter + Send + Sync + Clone,
    A: ArchetypeFilter + Send + Sync + Clone,
    C: ChunkFilter + Send + Sync + Clone,
{
    type Layout = L;
    type Archetype = A;
    type Chunk = C;

    fn static_filters(&self) -> (&Self::Layout, &Self::Archetype) {
        (&self.layout_filter, &self.archetype_filter)
    }

    fn filters(&mut self) -> (&Self::Layout, &Self::Archetype, &mut Self::Chunk) {
        (
            &self.layout_filter,
            &self.archetype_filter,
            &mut self.chunk_filter,
        )
    }

    fn into_filters(self) -> (Self::Layout, Self::Archetype, Self::Chunk) {
        (self.layout_filter, self.archetype_filter, self.chunk_filter)
    }
}

impl<L, A, C> std::ops::Not for EntityFilterTuple<L, A, C>
where
    L: LayoutFilter + std::ops::Not,
    L::Output: LayoutFilter,
    A: ArchetypeFilter + std::ops::Not,
    A::Output: ArchetypeFilter,
    C: ChunkFilter + std::ops::Not,
    C::Output: ChunkFilter,
{
    type Output = EntityFilterTuple<L::Output, A::Output, C::Output>;

    #[inline]
    fn not(self) -> Self::Output {
        EntityFilterTuple {
            layout_filter: !self.layout_filter,
            archetype_filter: !self.archetype_filter,
            chunk_filter: !self.chunk_filter,
        }
    }
}

impl<'a, L1, A1, C1, L2, A2, C2> std::ops::BitAnd<EntityFilterTuple<L2, A2, C2>>
    for EntityFilterTuple<L1, A1, C1>
where
    L1: LayoutFilter + std::ops::BitAnd<L2>,
    L1::Output: LayoutFilter,
    L2: LayoutFilter,
    A1: ArchetypeFilter + std::ops::BitAnd<A2>,
    A1::Output: ArchetypeFilter,
    A2: ArchetypeFilter,
    C1: ChunkFilter + std::ops::BitAnd<C2>,
    C1::Output: ChunkFilter,
    C2: ChunkFilter,
{
    type Output = EntityFilterTuple<L1::Output, A1::Output, C1::Output>;

    #[inline]
    fn bitand(self, rhs: EntityFilterTuple<L2, A2, C2>) -> Self::Output {
        EntityFilterTuple {
            layout_filter: self.layout_filter & rhs.layout_filter,
            archetype_filter: self.archetype_filter & rhs.archetype_filter,
            chunk_filter: self.chunk_filter & rhs.chunk_filter,
        }
    }
}

impl<'a, L1, A1, C1, L2, A2, C2> std::ops::BitOr<EntityFilterTuple<L2, A2, C2>>
    for EntityFilterTuple<L1, A1, C1>
where
    L1: LayoutFilter + std::ops::BitOr<L2>,
    L1::Output: LayoutFilter,
    L2: LayoutFilter,
    A1: ArchetypeFilter + std::ops::BitOr<A2>,
    A1::Output: ArchetypeFilter,
    A2: ArchetypeFilter,
    C1: ChunkFilter + std::ops::BitOr<C2>,
    C1::Output: ChunkFilter,
    C2: ChunkFilter,
{
    type Output = EntityFilterTuple<L1::Output, A1::Output, C1::Output>;

    #[inline]
    fn bitor(self, rhs: EntityFilterTuple<L2, A2, C2>) -> Self::Output {
        EntityFilterTuple {
            layout_filter: self.layout_filter | rhs.layout_filter,
            archetype_filter: self.archetype_filter | rhs.archetype_filter,
            chunk_filter: self.chunk_filter | rhs.chunk_filter,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Passthrough;

impl LayoutFilter for Passthrough {
    #[inline]
    fn matches_layout(&self, _: &[ComponentTypeId], _: &[TagTypeId]) -> Option<bool> { None }
}

impl ArchetypeFilter for Passthrough {
    #[inline]
    fn matches_archetype(&self, _: &ArchetypeTagsRef) -> Option<bool> { None }
}

impl ChunkFilter for Passthrough {
    #[inline]
    fn prepare(&mut self) {}

    #[inline]
    fn matches_chunk(&mut self, _: &Chunk) -> Option<bool> { None }
}

impl std::ops::Not for Passthrough {
    type Output = Passthrough;

    #[inline]
    fn not(self) -> Self::Output { self }
}

impl<Rhs> std::ops::BitAnd<Rhs> for Passthrough {
    type Output = Rhs;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output { rhs }
}

impl<Rhs> std::ops::BitOr<Rhs> for Passthrough {
    type Output = Rhs;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output { rhs }
}

#[derive(Debug, Clone)]
pub struct Any;

impl ActiveFilter for Any {}

impl LayoutFilter for Any {
    #[inline]
    fn matches_layout(&self, _: &[ComponentTypeId], _: &[TagTypeId]) -> Option<bool> { Some(true) }
}

impl ArchetypeFilter for Any {
    #[inline]
    fn matches_archetype(&self, _: &ArchetypeTagsRef) -> Option<bool> { Some(true) }
}

impl ChunkFilter for Any {
    #[inline]
    fn prepare(&mut self) {}

    #[inline]
    fn matches_chunk(&mut self, _: &Chunk) -> Option<bool> { Some(true) }
}

impl<Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for Any {
    type Output = Rhs;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output { rhs }
}

impl std::ops::BitAnd<Passthrough> for Any {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output { self }
}

impl<Rhs: ActiveFilter> std::ops::BitOr<Rhs> for Any {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Rhs) -> Self::Output { self }
}

impl std::ops::BitOr<Passthrough> for Any {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output { self }
}

/// A filter which negates `F`.
#[derive(Debug, Clone)]
pub struct Not<F> {
    pub filter: F,
}

impl<F> ActiveFilter for Not<F> {}

impl<F: LayoutFilter> LayoutFilter for Not<F> {
    #[inline]
    fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
        self.filter.matches_layout(components, tags).map(|x| !x)
    }
}

impl<F: ArchetypeFilter> ArchetypeFilter for Not<F> {
    #[inline]
    fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
        self.filter.matches_archetype(tags).map(|x| !x)
    }
}

impl<F: ChunkFilter> ChunkFilter for Not<F> {
    #[inline]
    fn prepare(&mut self) { self.filter.prepare() }

    #[inline]
    fn matches_chunk(&mut self, chunk: &Chunk) -> Option<bool> {
        self.filter.matches_chunk(chunk).map(|x| !x)
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
    fn bitand(self, _: Passthrough) -> Self::Output { self }
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
    fn bitor(self, _: Passthrough) -> Self::Output { self }
}

/// A filter which requires all filters within `T` match.
#[derive(Debug, Clone)]
pub struct And<T> {
    pub filters: T,
}

macro_rules! and_filter {
    ($head_ty:ident) => {
        impl_and_filter!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_and_filter!($head_ty, $( $tail_ty ),*);
        and_filter!($( $tail_ty ),*);
    );
}

macro_rules! impl_and_filter {
    ( $( $ty:ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for And<($( $ty, )*)> {}

        impl<$( $ty: LayoutFilter ),*> LayoutFilter for And<($( $ty, )*)> {
            #[inline]
            fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_and($ty.matches_layout(components, tags)); )*
                result
            }
        }

        impl<$( $ty: ArchetypeFilter ),*> ArchetypeFilter for And<($( $ty, )*)> {
            #[inline]
            fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_and($ty.matches_archetype(tags)); )*
                result
            }
        }

        impl<$( $ty: ChunkFilter ),*> ChunkFilter for And<($( $ty, )*)> {
            #[inline]
            fn prepare(&mut self) {
                #![allow(non_snake_case)]

                let ($( $ty, )*) = &mut self.filters;
                $( $ty.prepare(); )*
            }

            #[inline]
            fn matches_chunk(&mut self, chunk: &Chunk) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_and($ty.matches_chunk(chunk)); )*
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
    };
}

and_filter!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

/// A filter which requires any filters within `T` match.
#[derive(Debug, Clone)]
pub struct Or<T> {
    pub filters: T,
}

macro_rules! or_filter {
    ($head_ty:ident) => {
        impl_or_filter!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_or_filter!($head_ty, $( $tail_ty ),*);
        or_filter!($( $tail_ty ),*);
    );
}

macro_rules! impl_or_filter {
    ( $( $ty:ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for Or<($( $ty, )*)> {}

        impl<$( $ty: LayoutFilter ),*> LayoutFilter for Or<($( $ty, )*)> {
            #[inline]
            fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_or($ty.matches_layout(components, tags)); )*
                result
            }
        }

        impl<$( $ty: ArchetypeFilter ),*> ArchetypeFilter for Or<($( $ty, )*)> {
            #[inline]
            fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_or($ty.matches_archetype(tags)); )*
                result
            }
        }

        impl<$( $ty: ChunkFilter ),*> ChunkFilter for Or<($( $ty, )*)> {
            #[inline]
            fn prepare(&mut self) {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                $( $ty.prepare(); )*
            }

            #[inline]
            fn matches_chunk(&mut self, chunk: &Chunk) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_or($ty.matches_chunk(chunk)); )*
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

        impl<$( $ty ),*> std::ops::BitAnd<Passthrough> for Or<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitand(self, _: Passthrough) -> Self::Output {
                self
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for Or<($( $ty, )*)> {
            type Output = Or<(Self, Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                Or {
                    filters: (self, rhs),
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
    };
}

or_filter!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);
