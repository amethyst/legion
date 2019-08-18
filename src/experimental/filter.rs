use crate::experimental::storage::ArchetypeData;
use crate::experimental::storage::ChunkId;
use crate::experimental::storage::Component;
use crate::experimental::storage::ComponentStorage;
use crate::experimental::storage::ComponentTypeId;
use crate::experimental::storage::ComponentTypes;
use crate::experimental::storage::SliceVecIter;
use crate::experimental::storage::Tag;
use crate::experimental::storage::TagTypeId;
use crate::experimental::storage::TagTypes;
use std::collections::HashMap;
use std::iter::Repeat;
use std::iter::Take;
use std::marker::PhantomData;
use std::slice::Iter;

pub mod filter {
    ///! Contains functions for constructing filters.
    use super::*;

    /// Creates an entity data filter which includes chunks that contain
    /// entity data components of type `T`.
    pub fn component<T: Component>() -> ComponentFilter<T> {
        ComponentFilter::new()
    }

    /// Creates a shared data filter which includes chunks that contain
    /// shared data components of type `T`.
    pub fn tag<T: Tag>() -> TagFilter<T> {
        TagFilter::new()
    }

    /// Creates a shared data filter which includes chunks that contain
    /// specific shared data values.
    pub fn tag_value<'a, T: Tag>(data: &'a T) -> TagValueFilter<'a, T> {
        TagValueFilter::new(data)
    }

    /// Creates a filter which includes chunks for which entity data components
    /// of type `T` have changed since the filter was last executed.
    pub fn changed<T: Component>() -> ComponentChangedFilter<T> {
        ComponentChangedFilter::new()
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

pub trait Filter<'a, T> {
    type Iter: Iterator;

    fn collect(&self, source: &'a T) -> Self::Iter;
    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool>;
}

pub struct ArchetypeFilterData<'a> {
    pub component_types: &'a ComponentTypes,
    pub tag_types: &'a TagTypes,
}

pub struct ChunkFilterData<'a> {
    pub archetype_data: &'a ArchetypeData,
}

pub trait EntityFilter:
    for<'a> Filter<'a, ArchetypeFilterData<'a>> + for<'a> Filter<'a, ChunkFilterData<'a>>
{
}

impl<T> EntityFilter for T where
    T: for<'a> Filter<'a, ArchetypeFilterData<'a>> + for<'a> Filter<'a, ChunkFilterData<'a>>
{
}

#[derive(Debug)]
pub struct Passthrough;

impl<'a> Filter<'a, ArchetypeFilterData<'a>> for Passthrough {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: &'a ArchetypeFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.component_types.len())
    }

    fn is_match(&mut self, _: <Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl<'a> Filter<'a, ChunkFilterData<'a>> for Passthrough {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: &'a ChunkFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.archetype_data.len())
    }

    fn is_match(&mut self, _: <Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl std::ops::Not for Passthrough {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for Passthrough {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, Rhs: EntityFilter> std::ops::BitOr<Rhs> for Passthrough {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct Not<F> {
    filter: F,
}

impl<'a, T, F: Filter<'a, T>> Filter<'a, T> for Not<F> {
    type Iter = F::Iter;

    fn collect(&self, source: &'a T) -> Self::Iter {
        self.filter.collect(source)
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        self.filter.is_match(item).map(|x| !x)
    }
}

impl<'a, F: EntityFilter, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for Not<F> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, F: EntityFilter, Rhs: EntityFilter> std::ops::BitOr<Rhs> for Not<F> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct And<T> {
    filters: T,
}

macro_rules! impl_and_filter {
    ( $( $ty: ident => $ty2: ident ),* ) => {
        impl<'a, T, $( $ty: Filter<'a, T> ),*> Filter<'a, T> for And<($( $ty, )*)> {
            type Iter = itertools::Zip<( $( $ty::Iter ),* )>;

            fn collect(&self, source: &'a T) -> Self::Iter {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let iters = (
                    $( $ty.collect(source) ),*
                );
                itertools::multizip(iters)
            }

            fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let ($( $ty2, )*) = item;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_and($ty.is_match($ty2)); )*
                result
            }
        }

        impl<$( $ty: EntityFilter ),*> std::ops::Not for And<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty: EntityFilter ),*, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for And<($( $ty, )*)> {
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

        impl<$( $ty: EntityFilter ),*, Rhs: EntityFilter> std::ops::BitOr<Rhs> for And<($( $ty, )*)> {
            type Output = Or<(Self, Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                Or {
                    filters: (self, rhs),
                }
            }
        }
    }
}

impl_and_filter!(A => a, B => b);
impl_and_filter!(A => a, B => b, C => c);
impl_and_filter!(A => a, B => b, C => c, D => d);
impl_and_filter!(A => a, B => b, C => c, D => d, E => e);
impl_and_filter!(A => a, B => b, C => c, D => d, E => e, F => f);

pub struct Or<T> {
    filters: T,
}

macro_rules! impl_or_filter {
    ( $( $ty: ident => $ty2: ident ),* ) => {
        impl<'a, T, $( $ty: Filter<'a, T> ),*> Filter<'a, T> for Or<($( $ty, )*)> {
            type Iter = itertools::Zip<( $( $ty::Iter ),* )>;

            fn collect(&self, source: &'a T) -> Self::Iter {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let iters = (
                    $( $ty.collect(source) ),*
                );
                itertools::multizip(iters)
            }

            fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let ($( $ty2, )*) = item;
                let mut result: Option<bool> = None;
                $( result = result.coalesce_or($ty.is_match($ty2)); )*
                result
            }
        }

        impl<$( $ty: EntityFilter ),*> std::ops::Not for Or<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty: EntityFilter ),*, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for Or<($( $ty, )*)> {
            type Output = And<(Self, Rhs)>;

            #[inline]
            fn bitand(self, rhs: Rhs) -> Self::Output {
                And {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty: EntityFilter ),*, Rhs: EntityFilter> std::ops::BitOr<Rhs> for Or<($( $ty, )*)> {
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
    }
}

impl_or_filter!(A => a, B => b);
impl_or_filter!(A => a, B => b, C => c);
impl_or_filter!(A => a, B => b, C => c, D => d);
impl_or_filter!(A => a, B => b, C => c, D => d, E => e);
impl_or_filter!(A => a, B => b, C => c, D => d, E => e, F => f);

pub struct ComponentFilter<T>(PhantomData<T>);

impl<T: Component> ComponentFilter<T> {
    fn new() -> Self {
        ComponentFilter(PhantomData)
    }
}

impl<'a, T: Component> Filter<'a, ArchetypeFilterData<'a>> for ComponentFilter<T> {
    type Iter = SliceVecIter<'a, ComponentTypeId>;

    fn collect(&self, source: &ArchetypeFilterData<'a>) -> Self::Iter {
        source.component_types.iter()
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&ComponentTypeId::of::<T>()))
    }
}

impl<'a, T> Filter<'a, ChunkFilterData<'a>> for ComponentFilter<T> {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: &'a ChunkFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.archetype_data.len())
    }

    fn is_match(&mut self, _: <Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl<T> std::ops::Not for ComponentFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for ComponentFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitOr<Rhs> for ComponentFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct TagFilter<T>(PhantomData<T>);

impl<T: Tag> TagFilter<T> {
    fn new() -> Self {
        TagFilter(PhantomData)
    }
}

impl<'a, T: Tag> Filter<'a, ArchetypeFilterData<'a>> for TagFilter<T> {
    type Iter = SliceVecIter<'a, TagTypeId>;

    fn collect(&self, source: &ArchetypeFilterData<'a>) -> Self::Iter {
        source.tag_types.iter()
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&TagTypeId::of::<T>()))
    }
}

impl<'a, T> Filter<'a, ChunkFilterData<'a>> for TagFilter<T> {
    type Iter = Take<Repeat<()>>;

    fn collect(&self, source: &'a ChunkFilterData<'a>) -> Self::Iter {
        std::iter::repeat(()).take(source.archetype_data.len())
    }

    fn is_match(&mut self, _: <Self::Iter as Iterator>::Item) -> Option<bool> {
        None
    }
}

impl<T> std::ops::Not for TagFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for TagFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitOr<Rhs> for TagFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

pub struct TagValueFilter<'a, T> {
    value: &'a T,
}

impl<'a, T: Tag> TagValueFilter<'a, T> {
    fn new(value: &'a T) -> Self {
        TagValueFilter { value }
    }
}

impl<'a, T: Tag> Filter<'a, ArchetypeFilterData<'a>> for TagValueFilter<'a, T> {
    type Iter = SliceVecIter<'a, TagTypeId>;

    fn collect(&self, source: &ArchetypeFilterData<'a>) -> Self::Iter {
        source.tag_types.iter()
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&TagTypeId::of::<T>()))
    }
}

impl<'a, T: Tag> Filter<'a, ChunkFilterData<'a>> for TagValueFilter<'a, T> {
    type Iter = Iter<'a, T>;

    fn collect(&self, source: &'a ChunkFilterData<'a>) -> Self::Iter {
        unsafe {
            source
                .archetype_data
                .tags(&TagTypeId::of::<T>())
                .unwrap()
                .data_slice::<T>()
                .iter()
        }
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item == self.value)
    }
}

impl<'a, T> std::ops::Not for TagValueFilter<'a, T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for TagValueFilter<'a, T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T, Rhs: EntityFilter> std::ops::BitOr<Rhs> for TagValueFilter<'a, T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

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

impl<'a, T: Component> Filter<'a, ArchetypeFilterData<'a>> for ComponentChangedFilter<T> {
    type Iter = SliceVecIter<'a, ComponentTypeId>;

    fn collect(&self, source: &ArchetypeFilterData<'a>) -> Self::Iter {
        source.component_types.iter()
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
        Some(item.contains(&ComponentTypeId::of::<T>()))
    }
}

impl<'a, T: Component> Filter<'a, ChunkFilterData<'a>> for ComponentChangedFilter<T> {
    type Iter = Iter<'a, ComponentStorage>;

    fn collect(&self, source: &'a ChunkFilterData<'a>) -> Self::Iter {
        source.archetype_data.iter_component_chunks()
    }

    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> Option<bool> {
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

impl<'a, T: Component, Rhs: EntityFilter> std::ops::BitAnd<Rhs> for ComponentChangedFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component, Rhs: EntityFilter> std::ops::BitOr<Rhs> for ComponentChangedFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}
