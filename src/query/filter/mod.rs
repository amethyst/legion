use super::view::Fetch;
use crate::{storage::component::ComponentTypeId, world::WorldId};

pub mod and;
pub mod any;
pub mod component;
pub mod maybe_changed;
pub mod not;
pub mod or;
pub mod passthrough;
pub mod try_component;

pub mod filter_fns {
    use super::{
        any::Any, component::ComponentFilter, maybe_changed::ComponentChangedFilter,
        passthrough::Passthrough, try_component::TryComponentFilter, EntityFilterTuple,
    };
    use crate::storage::component::Component;

    pub fn component<T: Component>() -> EntityFilterTuple<ComponentFilter<T>, Passthrough> {
        Default::default()
    }

    pub fn maybe_changed<T: Component>(
    ) -> EntityFilterTuple<TryComponentFilter<T>, ComponentChangedFilter<T>> {
        Default::default()
    }

    pub fn any() -> EntityFilterTuple<Any, Any> { Default::default() }

    pub fn passthrough() -> EntityFilterTuple<Passthrough, Passthrough> { Default::default() }
}

pub enum FilterResult {
    Match(bool),
    Defer,
}

impl FilterResult {
    #[inline]
    pub fn coalesce_and(self, other: Self) -> Self {
        match self {
            Self::Match(success) => match other {
                Self::Match(other_success) => Self::Match(success && other_success),
                Self::Defer => Self::Match(success),
            },
            Self::Defer => other,
        }
    }

    #[inline]
    pub fn coalesce_or(self, other: Self) -> Self {
        match self {
            Self::Match(success) => match other {
                Self::Match(other_success) => Self::Match(success || other_success),
                Self::Defer => Self::Match(success),
            },
            Self::Defer => other,
        }
    }

    #[inline]
    pub fn is_pass(&self) -> bool {
        match self {
            Self::Match(success) => *success,
            Self::Defer => true,
        }
    }
}

impl std::ops::BitOr<FilterResult> for FilterResult {
    type Output = FilterResult;
    fn bitor(self, other: FilterResult) -> Self::Output { self.coalesce_or(other) }
}

impl std::ops::BitAnd<FilterResult> for FilterResult {
    type Output = FilterResult;
    fn bitand(self, other: FilterResult) -> Self::Output { self.coalesce_and(other) }
}

pub trait LayoutFilter: Send + Sync {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult;
}

pub trait DynamicFilter: Default + Send + Sync {
    fn prepare(&mut self, world: WorldId);
    fn matches_archetype<F: Fetch>(&mut self, fetch: &F) -> FilterResult;
}

/// A marker trait for filters that are not no-ops.
pub trait ActiveFilter {}

pub trait GroupMatcher {
    fn can_match_group() -> bool;
    fn group_components() -> Vec<ComponentTypeId>;
}

pub trait EntityFilter: Default + Send + Sync {
    type Layout: LayoutFilter + GroupMatcher + Default;
    type Dynamic: DynamicFilter;

    fn layout_filter(&self) -> &Self::Layout;
    fn filters(&mut self) -> (&Self::Layout, &mut Self::Dynamic);
}

impl<T: EntityFilter> LayoutFilter for T {
    fn matches_layout(
        &self,
        components: &[crate::storage::component::ComponentTypeId],
    ) -> FilterResult {
        self.layout_filter().matches_layout(components)
    }
}

impl<T: EntityFilter> DynamicFilter for T {
    fn prepare(&mut self, world: WorldId) {
        let (_, dynamic_filter) = self.filters();
        dynamic_filter.prepare(world);
    }

    fn matches_archetype<Fet: Fetch>(&mut self, fetch: &Fet) -> FilterResult {
        let (_, dynamic_filter) = self.filters();
        dynamic_filter.matches_archetype(fetch)
    }
}

impl<T: EntityFilter> GroupMatcher for T {
    fn can_match_group() -> bool { T::Layout::can_match_group() }
    fn group_components() -> Vec<ComponentTypeId> { T::Layout::group_components() }
}

#[derive(Clone, Default)]
pub struct EntityFilterTuple<L: LayoutFilter, F: DynamicFilter> {
    pub layout_filter: L,
    pub dynamic_filter: F,
}

impl<L: LayoutFilter, F: DynamicFilter> EntityFilterTuple<L, F> {
    pub fn new(layout_filter: L, dynamic_filter: F) -> Self {
        Self {
            layout_filter,
            dynamic_filter,
        }
    }
}

impl<L: LayoutFilter + GroupMatcher + Default, F: DynamicFilter> EntityFilter
    for EntityFilterTuple<L, F>
{
    type Layout = L;
    type Dynamic = F;

    fn layout_filter(&self) -> &Self::Layout { &self.layout_filter }

    fn filters(&mut self) -> (&Self::Layout, &mut Self::Dynamic) {
        (&self.layout_filter, &mut self.dynamic_filter)
    }
}

impl<L, F> std::ops::Not for EntityFilterTuple<L, F>
where
    L: LayoutFilter + std::ops::Not,
    L::Output: LayoutFilter,
    F: DynamicFilter + std::ops::Not,
    F::Output: DynamicFilter,
{
    type Output = EntityFilterTuple<L::Output, F::Output>;

    #[inline]
    fn not(self) -> Self::Output {
        EntityFilterTuple {
            layout_filter: !self.layout_filter,
            dynamic_filter: !self.dynamic_filter,
        }
    }
}

impl<'a, L1, F1, L2, F2> std::ops::BitAnd<EntityFilterTuple<L2, F2>> for EntityFilterTuple<L1, F1>
where
    L1: LayoutFilter + std::ops::BitAnd<L2>,
    L1::Output: LayoutFilter,
    L2: LayoutFilter,
    F1: DynamicFilter + std::ops::BitAnd<F2>,
    F1::Output: DynamicFilter,
    F2: DynamicFilter,
{
    type Output = EntityFilterTuple<L1::Output, F1::Output>;

    #[inline]
    fn bitand(self, rhs: EntityFilterTuple<L2, F2>) -> Self::Output {
        EntityFilterTuple {
            layout_filter: self.layout_filter & rhs.layout_filter,
            dynamic_filter: self.dynamic_filter & rhs.dynamic_filter,
        }
    }
}

impl<'a, L1, F1, L2, F2> std::ops::BitOr<EntityFilterTuple<L2, F2>> for EntityFilterTuple<L1, F1>
where
    L1: LayoutFilter + std::ops::BitOr<L2>,
    L1::Output: LayoutFilter,
    L2: LayoutFilter,
    F1: DynamicFilter + std::ops::BitOr<F2>,
    F1::Output: DynamicFilter,
    F2: DynamicFilter,
{
    type Output = EntityFilterTuple<L1::Output, F1::Output>;

    #[inline]
    fn bitor(self, rhs: EntityFilterTuple<L2, F2>) -> Self::Output {
        EntityFilterTuple {
            layout_filter: self.layout_filter | rhs.layout_filter,
            dynamic_filter: self.dynamic_filter | rhs.dynamic_filter,
        }
    }
}
