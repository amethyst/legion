use crate::experimental::storage::ArchetypeData;
use crate::experimental::storage::ComponentTypes;
use crate::experimental::storage::TagTypes;

pub trait Filter<'a, T> {
    type Iter: Iterator;

    fn collect(&self, source: &'a T) -> Self::Iter;
    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> bool;
}

pub struct ArchetypeFilterData<'a> {
    pub component_types: &'a ComponentTypes,
    pub tag_types: &'a TagTypes,
}

pub struct ChunkFilterData<'a> {
    pub archetype_data: &'a ArchetypeData,
}
