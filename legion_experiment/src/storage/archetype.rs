use super::{
    component::{Component, ComponentTypeId},
    UnknownComponentStorage,
};
use crate::entity::Entity;
use std::ops::{Index, IndexMut};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ArchetypeIndex(pub u32);

impl Index<ArchetypeIndex> for [Archetype] {
    type Output = Archetype;

    fn index(&self, index: ArchetypeIndex) -> &Self::Output {
        &self[index.0 as usize]
    }
}

impl IndexMut<ArchetypeIndex> for [Archetype] {
    fn index_mut(&mut self, index: ArchetypeIndex) -> &mut Self::Output {
        &mut self[index.0 as usize]
    }
}

impl Index<ArchetypeIndex> for Vec<Archetype> {
    type Output = Archetype;

    fn index(&self, index: ArchetypeIndex) -> &Self::Output {
        &self[index.0 as usize]
    }
}

impl IndexMut<ArchetypeIndex> for Vec<Archetype> {
    fn index_mut(&mut self, index: ArchetypeIndex) -> &mut Self::Output {
        &mut self[index.0 as usize]
    }
}

pub struct Archetype {
    entities: Vec<Entity>,
    layout: EntityLayout,
}

impl Archetype {
    pub fn new(layout: EntityLayout) -> Self {
        Self {
            layout,
            entities: Vec::new(),
        }
    }

    pub fn layout(&self) -> &EntityLayout {
        &self.layout
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn entities_mut(&mut self) -> &mut Vec<Entity> {
        &mut self.entities
    }
}

#[derive(Default, Debug)]
pub struct EntityLayout {
    components: Vec<ComponentTypeId>,
    component_constructors: Vec<fn() -> Box<dyn UnknownComponentStorage>>,
}

impl EntityLayout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_component<T: Component>(&mut self) {
        let type_id = ComponentTypeId::of::<T>();
        assert!(
            !self.components.contains(&type_id),
            "only one component of a given type may be attached to a single entity"
        );
        self.components.push(type_id);
        self.component_constructors
            .push(|| Box::new(T::Storage::default()));
    }

    pub fn component_types(&self) -> &[ComponentTypeId] {
        &self.components
    }

    pub fn component_constructors(&self) -> &[fn() -> Box<dyn UnknownComponentStorage>] {
        &self.component_constructors
    }

    pub fn has_component<T: Component>(&self) -> bool {
        self.components.contains(&ComponentTypeId::of::<T>())
    }
}
