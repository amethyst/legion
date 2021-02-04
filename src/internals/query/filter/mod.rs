//! Defines all filter types. Filters are a component of [queries](../index.html).

use super::view::Fetch;
use crate::internals::{storage::component::ComponentTypeId, world::WorldId};

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
    use crate::internals::storage::component::Component;

    /// Constructs a filter which requires that the entities have the given component.
    pub fn component<T: Component>() -> EntityFilterTuple<ComponentFilter<T>, Passthrough> {
        Default::default()
    }

    /// Constructs a filter which requires that the component cannot be certain to have not changed.
    ///
    /// This check is coarse grained and should be used to reject the majority of entities which have
    /// not changed, but not all entities passed by the filter are guaranteed to have been modified.
    pub fn maybe_changed<T: Component>(
    ) -> EntityFilterTuple<TryComponentFilter<T>, ComponentChangedFilter<T>> {
        Default::default()
    }

    /// Constructs a filter which passes all entities.
    pub fn any() -> EntityFilterTuple<Any, Any> {
        Default::default()
    }

    /// Constructs a filter which performs a no-op and defers to any filters it is combined with.
    pub fn passthrough() -> EntityFilterTuple<Passthrough, Passthrough> {
        Default::default()
    }
}

/// Indicates if an an archetype should be accepted or rejected.
pub enum FilterResult {
    /// The filter has made a decision, `true` for accept, `false` for reject.
    Match(bool),
    /// The filter has not made a decision, defer to other filters.
    Defer,
}

impl FilterResult {
    /// Combines the result with a logical and operator.
    #[inline]
    pub fn coalesce_and(self, other: Self) -> Self {
        match self {
            Self::Match(success) => {
                match other {
                    Self::Match(other_success) => Self::Match(success && other_success),
                    Self::Defer => Self::Match(success),
                }
            }
            Self::Defer => other,
        }
    }

    /// Combines the result with a logical or operator.
    #[inline]
    pub fn coalesce_or(self, other: Self) -> Self {
        match self {
            Self::Match(success) => {
                match other {
                    Self::Match(other_success) => Self::Match(success || other_success),
                    Self::Defer => Self::Match(success),
                }
            }
            Self::Defer => other,
        }
    }

    /// Returns `true` if the archetype should be accepted.
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
    fn bitor(self, other: FilterResult) -> Self::Output {
        self.coalesce_or(other)
    }
}

impl std::ops::BitAnd<FilterResult> for FilterResult {
    type Output = FilterResult;
    fn bitand(self, other: FilterResult) -> Self::Output {
        self.coalesce_and(other)
    }
}

/// A filter which selects based upon which component types are attached to an entity.
///
/// These filters should be idempotent and immutable.
pub trait LayoutFilter {
    /// Calculates the filter's result for the given entity layout.
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult;
}

/// A filter which selects based upon the data available in the archetype.
pub trait DynamicFilter: Default + Send + Sync {
    /// Prepares the filter to run.
    fn prepare(&mut self, world: WorldId);

    /// Calculates the filter's result for the given archetype data.
    fn matches_archetype<F: Fetch>(&mut self, fetch: &F) -> FilterResult;
}

/// A marker trait for filters that are not no-ops.
pub trait ActiveFilter {}

/// Allows a filter to determine if component optimization groups can
/// be used to accelerate queries that use this filter.
pub trait GroupMatcher {
    /// Returns `true` if the filter may potentially match a group.
    fn can_match_group() -> bool;

    /// Returns the components that are requried to be present in a group.
    fn group_components() -> Vec<ComponentTypeId>;
}

/// A combination of a [LayoutFilter](trait.LayoutFilter.html) and a
/// [DynamicFilter](trait.DynamicFilter.html).
pub trait EntityFilter: Default + Send + Sync {
    /// The layout filter type.
    type Layout: LayoutFilter + GroupMatcher + Default + Send + Sync;
    /// The dynamic filter type.
    type Dynamic: DynamicFilter;

    /// Returns a reference to the layout filter.
    fn layout_filter(&self) -> &Self::Layout;

    /// Returns a tuple of the layout and dynamic filters.
    fn filters(&mut self) -> (&Self::Layout, &mut Self::Dynamic);
}

impl<T: EntityFilter> LayoutFilter for T {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
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
    fn can_match_group() -> bool {
        T::Layout::can_match_group()
    }
    fn group_components() -> Vec<ComponentTypeId> {
        T::Layout::group_components()
    }
}

#[doc(hidden)]
#[derive(Clone, Default)]
pub struct EntityFilterTuple<L: LayoutFilter, F: DynamicFilter> {
    pub layout_filter: L,
    pub dynamic_filter: F,
}

impl<L, F> EntityFilter for EntityFilterTuple<L, F>
where
    L: LayoutFilter + GroupMatcher + Default + Send + Sync,
    F: DynamicFilter,
{
    type Layout = L;
    type Dynamic = F;

    fn layout_filter(&self) -> &Self::Layout {
        &self.layout_filter
    }

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
