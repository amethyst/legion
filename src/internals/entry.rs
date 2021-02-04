use std::sync::Arc;

use thiserror::Error;

use crate::internals::{
    entity::EntityLocation,
    insert::ArchetypeSource,
    query::filter::{FilterResult, LayoutFilter},
    storage::{
        archetype::{Archetype, EntityLayout},
        component::{Component, ComponentTypeId},
        ComponentStorage, Components, UnknownComponentStorage,
    },
    subworld::ComponentAccess,
    world::World,
};

/// An error type which describes why an attempt to retrieve a component failed.
#[derive(Error, Copy, Clone, Debug, PartialEq, Hash)]
pub enum ComponentError {
    /// The component was not found on the entity.
    #[error("the component {component_name} was not found on the entity")]
    NotFound {
        /// The type ID of the component.
        component_type: ComponentTypeId,
        /// The type name of the component.
        component_name: &'static str,
    },

    /// The world does not allow access to the component.
    #[error("the world does not declare appropriate access to {component_name}, \
            consider adding a query which contains `{component_name}` and this entity in its result set to the system, \
            or use `SystemBuilder::read_component` to declare global access over all entities")]
    Denied {
        /// The type ID of the component.
        component_type: ComponentTypeId,
        /// The type name of the component.
        component_name: &'static str,
    },
}

/// Provides safe read-only access to an entity's components.
pub struct EntryRef<'a> {
    pub(crate) location: EntityLocation,
    pub(crate) components: &'a Components,
    pub(crate) archetypes: &'a [Archetype],
    pub(crate) allowed_components: ComponentAccess<'a>,
}

impl<'a> EntryRef<'a> {
    pub(crate) fn new(
        location: EntityLocation,
        components: &'a Components,
        archetypes: &'a [Archetype],
        allowed_components: ComponentAccess<'a>,
    ) -> Self {
        Self {
            location,
            components,
            archetypes,
            allowed_components,
        }
    }

    /// Returns the entity's archetype.
    pub fn archetype(&self) -> &Archetype {
        &self.archetypes[self.location.archetype()]
    }

    /// Returns the entity's location.
    pub fn location(&self) -> EntityLocation {
        self.location
    }

    /// Returns a reference to one of the entity's components.
    pub fn into_component<T: Component>(self) -> Result<&'a T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_read(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// This function bypasses static borrow checking. The caller must ensure that the component reference
    /// will not be mutably aliased.
    pub unsafe fn into_component_unchecked<T: Component>(
        self,
    ) -> Result<&'a mut T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_write(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a reference to one of the entity's components.
    pub fn get_component<T: Component>(&self) -> Result<&T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_read(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// This function bypasses static borrow checking. The caller must ensure that the component reference
    /// will not be mutably aliased.
    pub unsafe fn get_component_unchecked<T: Component>(&self) -> Result<&mut T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_write(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }
}

/// Provides safe read and write access to an entity's components.
pub struct EntryMut<'a> {
    pub(crate) location: EntityLocation,
    pub(crate) components: &'a Components,
    pub(crate) archetypes: &'a [Archetype],
    pub(crate) allowed_components: ComponentAccess<'a>,
}

impl<'a> EntryMut<'a> {
    pub(crate) unsafe fn new(
        location: EntityLocation,
        components: &'a Components,
        archetypes: &'a [Archetype],
        allowed_components: ComponentAccess<'a>,
    ) -> Self {
        Self {
            location,
            components,
            archetypes,
            allowed_components,
        }
    }

    /// Returns the entity's archetype.
    pub fn archetype(&self) -> &Archetype {
        &self.archetypes[self.location.archetype()]
    }

    /// Returns the entity's location.
    pub fn location(&self) -> EntityLocation {
        self.location
    }

    /// Returns a reference to one of the entity's components.
    pub fn into_component<T: Component>(self) -> Result<&'a T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_read(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// This function bypasses static borrow checking. The caller must ensure that the component reference
    /// will not be mutably aliased.
    pub unsafe fn into_component_unchecked<T: Component>(
        self,
    ) -> Result<&'a mut T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self.allowed_components.allows_write(component_type) {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    pub fn into_component_mut<T: Component>(self) -> Result<&'a mut T, ComponentError> {
        // safety: we have exclusive access to the entry.
        // the world must ensure that mut entries handed out are unique
        unsafe { self.into_component_unchecked() }
    }

    /// Returns a reference to one of the entity's components.
    pub fn get_component<T: Component>(&self) -> Result<&T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self
            .allowed_components
            .allows_read(ComponentTypeId::of::<T>())
        {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// The caller must ensure that the component reference will not be mutably aliased.
    pub unsafe fn get_component_unchecked<T: Component>(&self) -> Result<&mut T, ComponentError> {
        let component_type = ComponentTypeId::of::<T>();
        if !self
            .allowed_components
            .allows_write(ComponentTypeId::of::<T>())
        {
            return Err(ComponentError::Denied {
                component_type,
                component_name: std::any::type_name::<T>(),
            });
        }

        let component = self.location.component();
        let archetype = self.location.archetype();
        self.components
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type,
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    pub fn get_component_mut<T: Component>(&mut self) -> Result<&mut T, ComponentError> {
        // safety: we have exclusive access to the entry.
        // the world must ensure that mut entries handed out are unique
        unsafe { self.get_component_unchecked() }
    }
}

/// Provides safe read and write access to an entity's components, and the ability to modify the entity.
pub struct Entry<'a> {
    location: EntityLocation,
    world: &'a mut World,
}

impl<'a> Entry<'a> {
    pub(crate) fn new(location: EntityLocation, world: &'a mut World) -> Self {
        Self { location, world }
    }

    /// Returns the entity's archetype.
    pub fn archetype(&self) -> &Archetype {
        &self.world.archetypes()[self.location.archetype()]
    }

    /// Returns the entity's location.
    pub fn location(&self) -> EntityLocation {
        self.location
    }

    /// Returns a reference to one of the entity's components.
    pub fn into_component<T: Component>(self) -> Result<&'a T, ComponentError> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type: ComponentTypeId::of::<T>(),
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    pub fn into_component_mut<T: Component>(self) -> Result<&'a mut T, ComponentError> {
        // safety: we have exclusive access to both the entry and the world
        unsafe { self.into_component_unchecked() }
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// This function bypasses static borrow checking. The caller must ensure that the component reference
    /// will not be mutably aliased.
    pub unsafe fn into_component_unchecked<T: Component>(
        self,
    ) -> Result<&'a mut T, ComponentError> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type: ComponentTypeId::of::<T>(),
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a reference to one of the entity's components.
    pub fn get_component<T: Component>(&self) -> Result<&T, ComponentError> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get(archetype))
            .and_then(move |slice| slice.into_slice().get(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type: ComponentTypeId::of::<T>(),
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Returns a mutable reference to one of the entity's components.
    pub fn get_component_mut<T: Component>(&mut self) -> Result<&mut T, ComponentError> {
        // safety: we have exclusive access to both the entry and the world
        unsafe { self.get_component_unchecked() }
    }

    /// Returns a mutable reference to one of the entity's components.
    ///
    /// # Safety
    /// This function bypasses static borrow checking. The caller must ensure that the component reference
    /// will not be mutably aliased.
    pub unsafe fn get_component_unchecked<T: Component>(&self) -> Result<&mut T, ComponentError> {
        let component = self.location.component();
        let archetype = self.location.archetype();
        self.world
            .components()
            .get_downcast::<T>()
            .and_then(move |storage| storage.get_mut(archetype))
            .and_then(move |slice| slice.into_slice().get_mut(component.0))
            .ok_or_else(|| {
                ComponentError::NotFound {
                    component_type: ComponentTypeId::of::<T>(),
                    component_name: std::any::type_name::<T>(),
                }
            })
    }

    /// Adds a new component to the entity.
    /// If the component already exists, its value will be replaced.
    pub fn add_component<T: Component>(&mut self, component: T) {
        if let Ok(comp) = self.get_component_mut::<T>() {
            *comp = component;
            return;
        }

        let target_arch = {
            let mut source = DynamicArchetype {
                base: self.world.archetypes()[self.location.archetype()]
                    .layout()
                    .clone(),
                add: &[ComponentTypeId::of::<T>()],
                add_constructors: &[|| Box::new(T::Storage::default())],
                remove: &[],
            };
            self.world.get_archetype_for_components(&mut source)
        };
        unsafe {
            let idx = self.world.transfer_archetype(
                self.location.archetype(),
                target_arch,
                self.location.component(),
            );
            self.world
                .components_mut()
                .get_downcast_mut::<T>()
                .unwrap()
                .extend_memcopy(target_arch, &component as *const T, 1);
            std::mem::forget(component);
            self.location = EntityLocation::new(target_arch, idx);
        };
    }

    /// Removes a component from the entity.
    /// Does nothing if the entity does not have the component.
    pub fn remove_component<T: Component>(&mut self) {
        if !self.archetype().layout().has_component::<T>() {
            return;
        }

        let target_arch = {
            let mut source = DynamicArchetype {
                base: self.world.archetypes()[self.location.archetype()]
                    .layout()
                    .clone(),
                add: &[],
                add_constructors: &[],
                remove: &[ComponentTypeId::of::<T>()],
            };
            self.world.get_archetype_for_components(&mut source)
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
    fn filter(&self) -> Self::Filter {
        self.clone()
    }
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
    use crate::internals::{
        query::{view::read::Read, IntoQuery},
        world::World,
    };

    #[test]
    #[allow(clippy::float_cmp)]
    fn add_component() {
        let mut world = World::default();

        let entities = world.extend(vec![(1usize, true), (2usize, false)]).to_vec();

        let mut query = Read::<f32>::query();
        let before = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(before.len(), 0);

        {
            let mut entry = world.entry(entities[0]).unwrap();
            assert!(entry.get_component::<f32>().is_err());
            entry.add_component(5f32);
            assert_eq!(entry.get_component::<f32>(), Ok(&5f32));
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
            let mut entry = world.entry(entities[0]).unwrap();
            assert_eq!(entry.get_component::<usize>(), Ok(&1usize));
            entry.remove_component::<usize>();
            assert!(entry.get_component::<usize>().is_err());
        }

        let after = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0], &2usize);
    }
}
