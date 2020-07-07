use super::{
    and::And, not::Not, or::Or, passthrough::Passthrough, ActiveFilter, FilterResult, GroupMatcher,
    LayoutFilter,
};
use crate::internals::storage::component::{Component, ComponentTypeId};
use std::marker::PhantomData;

/// A filter which matches `true` when the given component exists in the archetype.
#[derive(Debug)]
pub struct ComponentFilter<T: Component>(PhantomData<T>);

impl<T: Component> Default for ComponentFilter<T> {
    fn default() -> Self { ComponentFilter(PhantomData) }
}

impl<T: Component> Copy for ComponentFilter<T> {}
impl<T: Component> Clone for ComponentFilter<T> {
    fn clone(&self) -> Self { *self }
}

impl<T: Component> ActiveFilter for ComponentFilter<T> {}

impl<T: Component> GroupMatcher for ComponentFilter<T> {
    fn can_match_group() -> bool { true }
    fn group_components() -> Vec<ComponentTypeId> { vec![ComponentTypeId::of::<T>()] }
}

impl<T: Component> LayoutFilter for ComponentFilter<T> {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        FilterResult::Match(components.contains(&ComponentTypeId::of::<T>()))
    }
}

impl<T: Component> std::ops::Not for ComponentFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output { Not { filter: self } }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for ComponentFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitAnd<Passthrough> for ComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output { self }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for ComponentFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitOr<Passthrough> for ComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output { self }
}
