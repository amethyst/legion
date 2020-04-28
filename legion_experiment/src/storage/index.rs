use super::{
    archetype::{ArchetypeIndex, EntityLayout},
    component::ComponentTypeId,
    slicevec::SliceVec,
};
use crate::query::filter::LayoutFilter;

#[derive(Default)]
pub struct SearchIndex {
    component_layouts: SliceVec<ComponentTypeId>,
}

impl SearchIndex {
    pub fn new() -> Self {
        Self {
            component_layouts: SliceVec::default(),
        }
    }

    pub(crate) fn push(&mut self, archetype_layout: &EntityLayout) {
        self.component_layouts
            .push(archetype_layout.component_types().iter().copied());
    }

    pub fn search_from<'a, F: LayoutFilter>(
        &'a self,
        filter: &'a F,
        start: usize,
    ) -> impl Iterator<Item = ArchetypeIndex> + 'a {
        self.component_layouts
            .iter()
            .skip(start)
            .enumerate()
            .filter(move |(_, components)| filter.matches_layout(components).is_pass())
            .map(|(i, _)| ArchetypeIndex(i as u32))
    }

    pub fn search<'a, F: LayoutFilter>(
        &'a self,
        filter: &'a F,
    ) -> impl Iterator<Item = ArchetypeIndex> + 'a {
        self.search_from(filter, 0)
    }
}

// impl<T: LayoutFilter> LayoutFilter for &T {
//     fn matches_layout(&self, components: &[ComponentTypeId]) -> Option<bool> {
//         <T as LayoutFilter>::matches_layout(self, components)
//     }
// }
