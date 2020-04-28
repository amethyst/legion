use crate::{
    entity::EntityLocation,
    storage::{archetype::Archetype, component::Component, ComponentStorage, Components},
};

pub struct Entry<'a> {
    location: &'a EntityLocation,
    components: &'a Components,
    archetypes: &'a [Archetype],
}

impl<'a> Entry<'a> {
    pub(crate) fn new(
        location: &'a EntityLocation,
        components: &'a Components,
        archetypes: &'a [Archetype],
    ) -> Self {
        Self {
            location,
            components,
            archetypes,
        }
    }

    pub fn archetype(&self) -> &Archetype {
        &self.archetypes[self.location.archetype()]
    }

    pub fn location(&self) -> EntityLocation {
        *self.location
    }

    pub fn get_component<T: Component>(&self) -> Option<&T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
    }
}

pub struct EntryMut<'a> {
    location: &'a mut EntityLocation,
    components: &'a mut Components,
    archetypes: &'a mut [Archetype],
}

impl<'a> EntryMut<'a> {
    pub(crate) fn new(
        location: &'a mut EntityLocation,
        components: &'a mut Components,
        archetypes: &'a mut [Archetype],
    ) -> Self {
        Self {
            location,
            components,
            archetypes,
        }
    }

    pub fn archetype(&self) -> &Archetype {
        &self.archetypes[self.location.archetype()]
    }

    pub fn location(&self) -> EntityLocation {
        *self.location
    }

    pub fn get_component<T: Component>(&self) -> Option<&T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
    }

    pub fn get_component_mut<T: Component>(&mut self) -> Option<&mut T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| unsafe { storage.get_mut(archetype) })
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
    }

    pub fn add_component<T: Component>(&mut self, component: T) {
        todo!()
    }

    pub fn remove_component<T: Component>(&mut self) {
        todo!()
    }
}
