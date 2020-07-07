use super::{
    and::And, not::Not, or::Or, passthrough::Passthrough, ActiveFilter, DynamicFilter, FilterResult,
};
use crate::internals::{query::view::Fetch, storage::component::Component, world::WorldId};
use std::{collections::HashMap, marker::PhantomData};

/// A filter which performs course-grained change detection.
///
/// This filter will reject _most_ components which have not
/// been changed, but not all.
#[derive(Debug)]
pub struct ComponentChangedFilter<T: Component> {
    _phantom: PhantomData<T>,
    history: HashMap<WorldId, u64>,
    world: Option<WorldId>,
    threshold: u64,
    maximum: u64,
}

impl<T: Component> Default for ComponentChangedFilter<T> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
            history: Default::default(),
            world: None,
            threshold: 0,
            maximum: 0,
        }
    }
}

impl<T: Component> Clone for ComponentChangedFilter<T> {
    fn clone(&self) -> Self {
        Self {
            _phantom: PhantomData,
            history: self.history.clone(),
            world: None,
            threshold: 0,
            maximum: 0,
        }
    }
}

impl<T: Component> ActiveFilter for ComponentChangedFilter<T> {}

impl<T: Component> DynamicFilter for ComponentChangedFilter<T> {
    fn prepare(&mut self, world: WorldId) {
        if let Some(world) = self.world {
            self.history.insert(world, self.maximum);
        }

        self.world = Some(world);
        self.threshold = *self.history.entry(world).or_insert(0);
        self.maximum = self.threshold;
    }

    fn matches_archetype<Fet: Fetch>(&mut self, fetch: &Fet) -> FilterResult {
        if let Some(version) = fetch.version::<T>() {
            if version > self.maximum {
                self.maximum = version;
            }
            FilterResult::Match(version > self.threshold)
        } else {
            FilterResult::Defer
        }
    }
}

impl<T: Component> std::ops::Not for ComponentChangedFilter<T> {
    type Output = Not<Self>;

    #[inline]
    fn not(self) -> Self::Output { Not { filter: self } }
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
    fn bitand(self, _: Passthrough) -> Self::Output { self }
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
    fn bitor(self, _: Passthrough) -> Self::Output { self }
}
