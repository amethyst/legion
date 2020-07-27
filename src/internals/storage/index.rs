//! An index of archetype layouts used to accelerate query evaluation.

use super::{
    archetype::{ArchetypeIndex, EntityLayout},
    component::ComponentTypeId,
    slicevec::SliceVec,
};
use crate::internals::query::filter::LayoutFilter;

/// An index of archetype layouts used to accelerate query evaluation.
#[derive(Default, Debug)]
pub struct SearchIndex {
    component_layouts: SliceVec<ComponentTypeId>,
}

impl SearchIndex {
    pub(crate) fn push(&mut self, archetype_layout: &EntityLayout) {
        self.component_layouts
            .push(archetype_layout.component_types().iter().copied());
    }

    /// Returns an iterator over archetype indexes for archetypes which match the given layout filter,
    /// starting from the given index.
    pub fn search_from<'a, F: LayoutFilter>(
        &'a self,
        filter: &'a F,
        start: usize,
    ) -> impl Iterator<Item = ArchetypeIndex> + 'a {
        self.component_layouts
            .iter_from(start)
            .enumerate()
            .filter(move |(_, components)| filter.matches_layout(components).is_pass())
            .map(move |(i, _)| ArchetypeIndex((i + start) as u32))
    }

    /// Returns an iterator over archetype indexes for archetypes which match the given layout filter.
    pub fn search<'a, F: LayoutFilter>(
        &'a self,
        filter: &'a F,
    ) -> impl Iterator<Item = ArchetypeIndex> + 'a {
        self.search_from(filter, 0)
    }
}
