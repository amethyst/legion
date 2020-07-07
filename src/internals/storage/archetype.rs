//! Archetypes are sets of entities which all contain extactly the same
//! set of component types.
//!
//! Entities in the same archetype have all of their components stored next
//! to each other and in the same order, allowing their components to be
//! accessed zipped slices.
//!
//! Legion performs all entity filtering at the archetype level; decisions are
//! never made per-entity.

use super::{
    component::{Component, ComponentTypeId},
    UnknownComponentStorage,
};
use crate::internals::{
    entity::Entity,
    event::{Event, Subscriber, Subscribers},
    query::filter::{FilterResult, LayoutFilter},
};
use std::{
    ops::{Index, IndexMut},
    sync::Arc,
};

/// The index of an archetype in a world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct ArchetypeIndex(pub u32);

impl Index<ArchetypeIndex> for [Archetype] {
    type Output = Archetype;

    fn index(&self, index: ArchetypeIndex) -> &Self::Output { &self[index.0 as usize] }
}

impl IndexMut<ArchetypeIndex> for [Archetype] {
    fn index_mut(&mut self, index: ArchetypeIndex) -> &mut Self::Output {
        &mut self[index.0 as usize]
    }
}

impl Index<ArchetypeIndex> for Vec<Archetype> {
    type Output = Archetype;

    fn index(&self, index: ArchetypeIndex) -> &Self::Output { &self[index.0 as usize] }
}

impl IndexMut<ArchetypeIndex> for Vec<Archetype> {
    fn index_mut(&mut self, index: ArchetypeIndex) -> &mut Self::Output {
        &mut self[index.0 as usize]
    }
}

/// An archetype is a collection of entities which all have identical component types.
#[derive(Debug)]
pub struct Archetype {
    index: ArchetypeIndex,
    entities: Vec<Entity>,
    layout: Arc<EntityLayout>,
    subscribers: Subscribers,
}

impl Archetype {
    pub(crate) fn new(
        index: ArchetypeIndex,
        layout: EntityLayout,
        mut subscribers: Subscribers,
    ) -> Self {
        subscribers.send(Event::ArchetypeCreated(index));
        Self {
            index,
            layout: Arc::new(layout),
            entities: Vec::new(),
            subscribers,
        }
    }

    /// Returns the index of the archetype.
    pub fn index(&self) -> ArchetypeIndex { self.index }

    /// Returns the layout of the archetype, describing which components are attached
    /// to all entities contained within the archetype.
    pub fn layout(&self) -> &Arc<EntityLayout> { &self.layout }

    /// Returns a slice of entity IDs for all entities which belong to the archetype.
    pub fn entities(&self) -> &[Entity] { &self.entities }

    pub(crate) fn push(&mut self, entity: Entity) {
        self.entities.push(entity);
        self.subscribers
            .send(Event::EntityInserted(entity, self.index));
    }

    pub(crate) fn extend(&mut self, entities: impl IntoIterator<Item = Entity>) {
        let start = self.entities.len();
        self.entities.extend(entities);
        for entity in &self.entities[start..] {
            self.subscribers
                .send(Event::EntityInserted(*entity, self.index));
        }
    }

    pub(crate) fn swap_remove(&mut self, entity_index: usize) -> Entity {
        let removed = self.entities.swap_remove(entity_index);
        self.subscribers
            .send(Event::EntityRemoved(removed, self.index));
        removed
    }

    pub(crate) fn subscribe(&mut self, subscriber: Subscriber) {
        subscriber.send(Event::ArchetypeCreated(self.index));
        for entity in &self.entities {
            subscriber.send(Event::EntityInserted(*entity, self.index));
        }
        self.subscribers.push(subscriber);
    }

    pub(crate) fn drain(&mut self) -> Vec<Entity> {
        let mut entities = Vec::new();
        std::mem::swap(&mut self.entities, &mut entities);
        for entity in &entities {
            self.subscribers
                .send(Event::EntityRemoved(*entity, self.index));
        }
        entities
    }
}

impl Drop for Archetype {
    fn drop(&mut self) {
        for entity in &self.entities {
            self.subscribers
                .send(Event::EntityRemoved(*entity, self.index));
        }
    }
}

/// Describes the component types which are attached to an entity.
#[derive(Default, Debug, Clone)]
pub struct EntityLayout {
    components: Vec<ComponentTypeId>,
    component_constructors: Vec<fn() -> Box<dyn UnknownComponentStorage>>,
}

impl EntityLayout {
    /// Constructs a new entity layout.
    pub fn new() -> Self { Self::default() }

    /// Adds a new component to the layout.
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

    /// Adds a new component to the layout.
    pub unsafe fn register_component_raw(
        &mut self,
        type_id: ComponentTypeId,
        f: fn() -> Box<dyn UnknownComponentStorage>,
    ) {
        assert!(
            !self.components.contains(&type_id),
            "only one component of a given type may be attached to a single entity"
        );
        self.components.push(type_id);
        self.component_constructors.push(f);
    }

    /// Returns a slice of component type IDs for the components inside the layout.
    pub fn component_types(&self) -> &[ComponentTypeId] { &self.components }

    /// Returns a slice of storage constructors for each component type in the layout.
    #[doc(hidden)]
    pub fn component_constructors(&self) -> &[fn() -> Box<dyn UnknownComponentStorage>] {
        &self.component_constructors
    }

    /// Returns `true` if the layout contains the given component type.
    pub fn has_component<T: Component>(&self) -> bool {
        self.has_component_by_id(ComponentTypeId::of::<T>())
    }

    /// Returns `true` if the layout contains the given component type.
    pub fn has_component_by_id(&self, type_id: ComponentTypeId) -> bool {
        self.components.contains(&type_id)
    }
}

impl LayoutFilter for EntityLayout {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        FilterResult::Match(
            components.len() == self.components.len()
                && self.components.iter().all(|t| components.contains(t)),
        )
    }
}
