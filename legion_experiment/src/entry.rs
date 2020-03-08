use crate::borrow::{RefMap, RefMapMut};
use crate::entity::{EntityLocation, Locations};
use crate::storage::archetype::EntityTypeLayout;
use crate::storage::components::{Component, ComponentTypeId};
use crate::storage::tags::{Tag, TagTypeId};
use crate::storage::Storage;

pub struct Entry<'a> {
    location: EntityLocation,
    storage: &'a Storage,
}

impl<'a> Entry<'a> {
    pub(crate) fn new(location: EntityLocation, storage: &'a Storage) -> Self {
        Self { location, storage }
    }

    pub fn layout(&self) -> &EntityTypeLayout { self.storage[self.location.archetype()].layout() }

    pub fn location(&self) -> EntityLocation { self.location }

    pub fn get_component<T: Component>(&self) -> Option<RefMap<&T>> {
        let type_id = ComponentTypeId::of::<T>();
        let chunk = &self.storage[self.location.archetype()][self.location.chunk()];
        let components = chunk.components().get(type_id)?;
        Some(
            unsafe { components.get_slice::<T>() }
                .map_into(|slice| &slice[self.location.component().0]),
        )
    }

    pub fn get_tag<T: Tag>(&self) -> Option<&T> {
        let arch = &self.storage[self.location.archetype()];
        arch.tags().get()
    }
}

pub struct EntryMut<'a> {
    location: EntityLocation,
    storage: &'a mut Storage,
    entity_locations: &'a mut Locations,
}

impl<'a> EntryMut<'a> {
    pub(crate) fn new(
        location: EntityLocation,
        storage: &'a mut Storage,
        entity_locations: &'a mut Locations,
    ) -> Self {
        Self {
            location,
            storage,
            entity_locations,
        }
    }

    pub fn layout(&self) -> &EntityTypeLayout { self.storage[self.location.archetype()].layout() }

    pub fn location(&self) -> EntityLocation { self.location }

    pub fn get_component<T: Component>(&self) -> Option<RefMap<&T>> {
        let type_id = ComponentTypeId::of::<T>();
        let chunk = &self.storage[self.location.archetype()][self.location.chunk()];
        let components = chunk.components().get(type_id)?;
        Some(
            unsafe { components.get_slice::<T>() }
                .map_into(|slice| &slice[self.location.component().0]),
        )
    }

    pub fn get_component_mut<T: Component>(&mut self) -> Option<RefMapMut<&mut T>> {
        let type_id = ComponentTypeId::of::<T>();
        let chunk = &self.storage[self.location.archetype()][self.location.chunk()];
        let components = chunk.components().get(type_id)?;
        Some(
            unsafe { components.get_slice_mut::<T>() }
                .map_into(|slice| &mut slice[self.location.component().0]),
        )
    }

    pub fn get_tag<T: Tag>(&self) -> Option<&T> {
        let arch = &self.storage[self.location.archetype()];
        arch.tags().get()
    }

    pub fn add_component<T: Component>(&mut self, component: T) {}

    pub fn remove_component<T: Component>(&mut self) {}

    pub fn add_tag<T: Tag>(&mut self, tag: T) {}

    pub fn remove_tag<T: Tag>(&mut self) {}
}
