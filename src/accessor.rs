//! Entity accessor types, used to access  arbitrary components for an entity.

use crate::entity::{EntityLocation, Entity};
use crate::storage::{ComponentStorage, Component, ComponentTypeId};
use crate::world::World;
use crate::borrow::{Ref, Shared, RefMut, Exclusive};
use std::ops::Deref;

struct EntityAccessorInner<'data> {
    location: EntityLocation,
    entity: Entity,
    chunk: &'data ComponentStorage,
}

impl <'data> EntityAccessorInner<'data> {
    fn new(world: &'data World, entity: Entity) -> Option<Self> {
        if !world.is_alive(entity) {
            return None;
        }

        let location = world.entity_allocator.get_location(entity.index())?;
        let archetype = world.storage().archetypes().get(location.archetype())?;
        let chunk = archetype
            .chunksets()
            .get(location.set())?
            .get(location.chunk())?;

        Some(Self {
            location,
            entity,
            chunk,
        })
    }

    fn get<T: Component>(&self) -> Option<Ref<'data, Shared, T>> {
        let (slice_borrow, slice) = unsafe {
            self.chunk
                .components(ComponentTypeId::of::<T>())?
                .data_slice::<T>()
                .deconstruct()
        };
        let component = slice.get(self.location.component())?;

        Some(Ref::new(slice_borrow, component))
    }

    unsafe fn get_mut_unchecked<T: Component>(&self) -> Option<RefMut<'data, Exclusive, T>> {
        let (slice_borrow, slice) = self.chunk
                .components(ComponentTypeId::of::<T>())?
                .data_slice_mut::<T>()
                .deconstruct();
        let component = slice.get_mut(self.location.component())?;

        Some(RefMut::new(slice_borrow, component))
    }
}

/// An entity accessor.
///
/// This type can be used to access arbitrary components
/// for a given entity through a query. It can be obtained
/// through `Query::find_accessor` and `World::get_accessor`.
///
/// This type is useful if, for example, you want to pass
/// an entity and all its components to a function without
/// knowing which components need to be accessed.
///
/// Note that this type only provides immutable access to components; if you
/// would like mutable access, see `EntityAccessorMut`.
pub struct EntityAccessor<'data> {
    inner: EntityAccessorInner<'data>,
}

impl <'data> EntityAccessor<'data> {
    pub(crate) fn new(world: &'data World, entity: Entity) -> Option<Self> {
        Some(Self {
            inner: EntityAccessorInner::new(world, entity)?,
        })
    }

    /// Retrieves a component for this entity, returning it. Returns
    /// `None` if the entity does not have this component.
    pub fn get<T: Component>(&self) -> Option<Ref<'data, Shared, T>> {
        self.inner.get()
    }

    /// Returns the entity accessed by this `EntityAccessor`.
    pub fn entity(&self) -> Entity {
        self.inner.entity
    }
}

/// A mutable entity accessor.
///
/// This type can be used to access arbitrary components
/// for a given entity through a query. It can be obtained
/// through `Query::find_accessor_mut` and `World::get_acessor_mut`.
///
/// This type is useful if, for example, you want to pass
/// an entity and all its components to a function without
/// knowing which components need to be accessed.
///
/// Unlike `EntityAccessor`, this type allows mutable access to components.
pub struct EntityAccessorMut<'data> {
    // We store an immutable accessor so we can deref
    // to `EntityAccessor`. This allows users to pass
    // an `EntityAccessorMut` to a function accepting
    // `&EntityAccessor`.
    inner: EntityAccessor<'data>,
}

impl <'data> EntityAccessorMut<'data> {
    pub(crate) unsafe fn new(world: &'data World, entity: Entity) -> Option<Self> {
        Some(Self {
            inner: EntityAccessor::new(world, entity)?,
        })
    }

    /// Retrieves a component for this entity, returning it. Returns
    /// `None` if the entity does not have this component.
    pub fn get<T: Component>(&self) -> Option<Ref<'data, Shared, T>> {
        self.inner.get()
    }

    /// Mutably retrieves a component for this entity, returning it. Returns
    /// `None` if the entity does not have this component.
    pub fn get_mut<T: Component>(&mut self) -> Option<RefMut<'data, Exclusive, T>> {
        // Safety: mutable access to `self` ensures uniqueness of borrowed
        // component.
        unsafe { self.inner.inner.get_mut_unchecked() }
    }

    /// Mutably retrieves a component for this entity, returning it. Returns
    /// `None` if the entity does not have this component.
    ///
    /// # Safety
    /// No runtime checks are performed to ensure that the reference to the
    /// component is exclusive. Having multiple mutable references to the
    /// same component is undefined behavior.
    ///
    /// Unlike `get_mut``, this function does not require mutable access to `self`,
    /// so the compiler does not catch safety issues,
    pub unsafe fn get_mut_unchecked<T: Component>(&self) -> Option<RefMut<'data, Exclusive, T>> {
        self.inner.inner.get_mut_unchecked()
    }

    /// Returns the entity accessed by this `EntityAccessor`.
    pub fn entity(&self) -> Entity {
        self.inner.entity()
    }
}

impl <'data> Deref for EntityAccessorMut<'data> {
    type Target = EntityAccessor<'data>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use crate::world::Universe;

    #[test]
    fn accessor() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let entity = world.insert((), vec![("test", 1)])[0];
        let accessor = world.get_accessor(entity).unwrap();

        assert_eq!(*accessor.get::<&'static str>().unwrap(), "test");
        assert_eq!(*accessor.get::<i32>().unwrap(), 1);

        let mut accessor = world.get_accessor_mut(entity).unwrap();

        *accessor.get_mut::<i32>().unwrap() += 1;

        assert_eq!(*world.get_component::<i32>(entity).unwrap(), 2);
    }
}
