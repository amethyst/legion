use std::marker::PhantomData;

use super::{
    and::And, not::Not, or::Or, passthrough::Passthrough, ActiveFilter, FilterResult, GroupMatcher,
    LayoutFilter,
};
use crate::internals::storage::component::{Component, ComponentTypeId};

/// A filter which matches `true` if the entity has the given component,
/// else it will defer.
#[derive(Debug)]
pub struct TryComponentFilter<T: Component>(PhantomData<T>);

impl<T: Component> Default for TryComponentFilter<T> {
    fn default() -> Self {
        TryComponentFilter(PhantomData)
    }
}

impl<T: Component> Copy for TryComponentFilter<T> {}
impl<T: Component> Clone for TryComponentFilter<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> ActiveFilter for TryComponentFilter<T> {}

impl<T: Component> GroupMatcher for TryComponentFilter<T> {
    fn can_match_group() -> bool {
        false
    }
    fn group_components() -> Vec<ComponentTypeId> {
        vec![]
    }
}

impl<T: Component> LayoutFilter for TryComponentFilter<T> {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        let contains = components.contains(&ComponentTypeId::of::<T>());
        if contains {
            FilterResult::Match(true)
        } else {
            FilterResult::Defer
        }
    }
}

impl<T: Component> std::ops::Not for TryComponentFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output {
        Not { filter: self }
    }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for TryComponentFilter<T> {
    type Output = And<(Self, Rhs)>;

    #[inline]
    fn bitand(self, rhs: Rhs) -> Self::Output {
        And {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitAnd<Passthrough> for TryComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitand(self, _: Passthrough) -> Self::Output {
        self
    }
}

impl<'a, T: Component, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for TryComponentFilter<T> {
    type Output = Or<(Self, Rhs)>;

    #[inline]
    fn bitor(self, rhs: Rhs) -> Self::Output {
        Or {
            filters: (self, rhs),
        }
    }
}

impl<'a, T: Component> std::ops::BitOr<Passthrough> for TryComponentFilter<T> {
    type Output = Self;

    #[inline]
    fn bitor(self, _: Passthrough) -> Self::Output {
        self
    }
}
