use super::{
    and::And, or::Or, passthrough::Passthrough, ActiveFilter, DynamicFilter, FilterResult,
    GroupMatcher, LayoutFilter,
};
use crate::{query::view::Fetch, storage::ComponentTypeId, world::WorldId};

/// A filter which negates `F`.
#[derive(Debug, Clone, Default)]
pub struct Not<F> {
    pub(super) filter: F,
}

impl<F> ActiveFilter for Not<F> {}

impl<F> GroupMatcher for Not<F> {
    fn can_match_group() -> bool { false }
    fn group_components() -> Vec<ComponentTypeId> { vec![] }
}

impl<F: LayoutFilter> LayoutFilter for Not<F> {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        match self.filter.matches_layout(components) {
            FilterResult::Match(success) => FilterResult::Match(!success),
            FilterResult::Defer => FilterResult::Defer,
        }
    }
}

impl<F: DynamicFilter> DynamicFilter for Not<F> {
    fn prepare(&mut self, world: WorldId) { self.filter.prepare(world); }

    fn matches_archetype<T: Fetch>(&mut self, fetch: &T) -> FilterResult {
        match self.filter.matches_archetype(fetch) {
            FilterResult::Match(success) => FilterResult::Match(!success),
            FilterResult::Defer => FilterResult::Defer,
        }
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
