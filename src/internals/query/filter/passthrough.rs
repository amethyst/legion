use super::{DynamicFilter, FilterResult, GroupMatcher, LayoutFilter};
use crate::internals::{query::view::Fetch, storage::component::ComponentTypeId, world::WorldId};

/// A filter which always defers.
#[derive(Debug, Clone, Default)]
pub struct Passthrough;

impl GroupMatcher for Passthrough {
    fn can_match_group() -> bool { true }
    fn group_components() -> Vec<ComponentTypeId> { vec![] }
}

impl LayoutFilter for Passthrough {
    fn matches_layout(&self, _: &[ComponentTypeId]) -> FilterResult { FilterResult::Defer }
}

impl DynamicFilter for Passthrough {
    fn prepare(&mut self, _: WorldId) {}
    fn matches_archetype<F: Fetch>(&mut self, _: &F) -> FilterResult { FilterResult::Defer }
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
