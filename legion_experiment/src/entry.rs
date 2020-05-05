use crate::{
    entity::EntityLocation,
    insert::ArchetypeSource,
    query::filter::{FilterResult, LayoutFilter},
    storage::{
        archetype::{Archetype, EntityLayout},
        component::{Component, ComponentTypeId},
        ComponentStorage, Components, UnknownComponentStorage,
    },
    world::World,
};
use std::sync::Arc;

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

    pub fn archetype(&self) -> &Archetype { &self.archetypes[self.location.archetype()] }

    pub fn location(&self) -> EntityLocation { *self.location }

    pub fn get_component<T: Component>(&self) -> Option<&T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
    }

    pub unsafe fn get_component_unchecked<T: Component>(&self) -> Option<&mut T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
    }
}

pub struct EntryMut<'a> {
    location: EntityLocation,
    world: &'a mut World,
}

impl<'a> EntryMut<'a> {
    pub(crate) fn new(location: EntityLocation, world: &'a mut World) -> Self {
        Self { location, world }
    }

    pub fn archetype(&self) -> &Archetype { &self.world.archetypes()[self.location.archetype()] }

    pub fn location(&self) -> EntityLocation { self.location }

    pub fn get_component<T: Component>(&self) -> Option<&T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
    }

    pub fn get_component_mut<T: Component>(&mut self) -> Option<&mut T> {
        unsafe { self.get_component_unchecked() }
    }

    pub unsafe fn get_component_unchecked<T: Component>(&self) -> Option<&mut T> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
    }

    pub fn add_component<T: Component>(&mut self, component: T) {
        let target_arch = {
            let mut source = DynamicArchetype {
                base: self.world.archetypes()[self.location.archetype()]
                    .layout()
                    .clone(),
                add: &[ComponentTypeId::of::<T>()],
                add_constructors: &[|| Box::new(T::Storage::default())],
                remove: &[],
            };
            self.world.get_archetype(&mut source)
        };
        unsafe {
            let idx = self.world.transfer_archetype(
                self.location.archetype(),
                target_arch,
                self.location.component(),
            );
            let epoch = self.world.epoch();
            self.world
                .components_mut()
                .get_downcast_mut::<T>()
                .unwrap()
                .extend_memcopy(epoch, target_arch, &component as *const T, 1);
            std::mem::forget(component);
            self.location = EntityLocation::new(target_arch, idx);
        };
    }

    pub fn remove_component<T: Component>(&mut self) {
        let target_arch = {
            let mut source = DynamicArchetype {
                base: self.world.archetypes()[self.location.archetype()]
                    .layout()
                    .clone(),
                add: &[],
                add_constructors: &[],
                remove: &[ComponentTypeId::of::<T>()],
            };
            self.world.get_archetype(&mut source)
        };
        unsafe {
            let idx = self.world.transfer_archetype(
                self.location.archetype(),
                target_arch,
                self.location.component(),
            );
            self.location = EntityLocation::new(target_arch, idx);
        };
    }
}

#[derive(Clone)]
struct DynamicArchetype<'a> {
    base: Arc<EntityLayout>,
    add: &'a [ComponentTypeId],
    add_constructors: &'a [fn() -> Box<dyn UnknownComponentStorage>],
    remove: &'a [ComponentTypeId],
}

impl<'a> LayoutFilter for DynamicArchetype<'a> {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        let base_components = self.base.component_types();
        FilterResult::Match(
            components.len() == (base_components.len() + self.add.len() - self.remove.len())
                && components.iter().all(|t| {
                    !self.remove.contains(t)
                        && (base_components.iter().any(|x| x == t)
                            || self.add.iter().any(|x| x == t))
                }),
        )
    }
}

impl<'a> ArchetypeSource for DynamicArchetype<'a> {
    type Filter = Self;
    fn filter(&self) -> Self::Filter { self.clone() }
    fn layout(&mut self) -> EntityLayout {
        let mut layout = EntityLayout::new();
        for (type_id, constructor) in self
            .base
            .component_types()
            .iter()
            .zip(self.base.component_constructors())
        {
            if !self.remove.contains(type_id) {
                unsafe { layout.register_component_raw(*type_id, *constructor) };
            }
        }
        for (type_id, constructor) in self.add.iter().zip(self.add_constructors.iter()) {
            unsafe { layout.register_component_raw(*type_id, *constructor) };
        }
        layout
    }
}

#[cfg(test)]
mod test {
    use crate::{query::view::read::Read, query::IntoQuery, world::World};

    #[test]
    fn add_component() {
        let mut world = World::default();

        let entities = world.extend(vec![(1usize, true), (2usize, false)]).to_vec();

        let mut query = Read::<f32>::query();
        let before = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(before.len(), 0);

        {
            let mut entry = world.entry_mut(entities[0]).unwrap();
            assert_eq!(entry.get_component::<f32>(), None);
            entry.add_component(5f32);
            assert_eq!(entry.get_component::<f32>(), Some(&5f32));
        }

        let after = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0], &5f32);
    }

    #[test]
    fn remove_component() {
        let mut world = World::default();

        let entities = world.extend(vec![(1usize, true), (2usize, false)]).to_vec();

        let mut query = Read::<usize>::query();
        let before = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(before.len(), 2);
        assert_eq!(before[0], &1usize);
        assert_eq!(before[1], &2usize);

        {
            let mut entry = world.entry_mut(entities[0]).unwrap();
            assert_eq!(entry.get_component::<usize>(), Some(&1usize));
            entry.remove_component::<usize>();
            assert_eq!(entry.get_component::<usize>(), None);
        }

        let after = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0], &2usize);
    }
}
