use crate::{
    internals::entity::EntityHasher, internals::serialize::CustomEntitySerializer, world::Allocate,
    Entity,
};
use serde::{Deserialize, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashMap};
use thiserror::Error;
use uuid::Uuid;

/// Describes how to serialize and deserialize a runtime `Entity` ID.
pub trait EntitySerializer {
    /// Serializes an `Entity` by constructing the serializable representation
    /// and passing it into `serialize_fn`.
    fn serialize(&self, entity: Entity, serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize));

    /// Deserializes an `Entity`.
    fn deserialize(
        &self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error>;
}

thread_local! {
    static SERIALIZER: RefCell<Option<&'static dyn EntitySerializer>> = RefCell::new(None);
}

/// Runs the provided function with this canon as context for Entity (de)serialization.
pub fn run_as_context<F: FnOnce() -> R, R>(context: &dyn EntitySerializer, f: F) -> R {
    struct SerializerGuard<'a> {
        cell: &'a RefCell<Option<&'static dyn EntitySerializer>>,
        prev: Option<&'static dyn EntitySerializer>,
    }

    impl Drop for SerializerGuard<'_> {
        fn drop(&mut self) {
            // swap context back out of TLS, putting back what we took out.
            let mut existing = self.cell.borrow_mut();
            *existing = self.prev;
        }
    }

    SERIALIZER.with(|cell| {
        // swap context into TLS
        let _serializer_guard = {
            let mut existing = cell.borrow_mut();
            // Safety? Hmm
            // This &'static reference is only stored in the TLS for the duration of this function,
            // meaning it will be valid for all TLS uses as long as it's not copied anywhere else.
            // Can't express this lifetime in a TLS-static, so that's why this unsafe block needs to exist.
            //
            // If `f` unwinds, the SerializerGuard destructor will restore the previous value of
            // SERIALIZER, so there's no risk of leaving a dangling reference.
            let hacked_context = unsafe {
                core::mem::transmute::<&dyn EntitySerializer, &'static dyn EntitySerializer>(
                    context,
                )
            };
            let prev = std::mem::replace(&mut *existing, Some(hacked_context));
            SerializerGuard { cell, prev }
        };

        (f)()
    })
}

impl Serialize for Entity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SERIALIZER.with(|cell| {
            let mut entity_serializer = cell.borrow_mut();
            let mut result = None;
            let mut serializer = Some(serializer);
            let result_ref = &mut result;
            entity_serializer
                .as_mut()
                .expect("No entity serializer set")
                .serialize(*self, &mut move |serializable| {
                    *result_ref = Some(erased_serde::serialize(
                        serializable,
                        serializer
                            .take()
                            .expect("serialize can only be called once"),
                    ));
                });
            result.unwrap()
        })
    }
}

impl<'de> Deserialize<'de> for Entity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        SERIALIZER.with(|cell| {
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            let mut entity_serializer = cell.borrow_mut();
            entity_serializer
                .as_mut()
                .expect("No entity serializer set")
                .deserialize(&mut deserializer)
                .map_err(D::Error::custom)
        })
    }
}

/// A 16 byte UUID which uniquely identifies an entity.
pub type EntityName = [u8; 16];

/// Error returned on unsucessful attempt to canonize an entity.
#[derive(Error, Debug, Copy, Clone, PartialEq, Hash)]
pub enum CanonizeError {
    /// The entity already exists bound to a different name.
    #[error("the entity is already bound to name {0:?}")]
    EntityAlreadyBound(Entity, EntityName),
    /// The name already exists bound to a different entity.
    #[error("the name is already bound to a different entity")]
    NameAlreadyBound(Entity, EntityName),
}

/// Contains the canon names of entities.
#[derive(Default, Debug)]
pub struct Canon {
    to_name: HashMap<Entity, EntityName, EntityHasher>,
    to_id: HashMap<EntityName, Entity>,
    allocator: Allocate,
}

impl Canon {
    /// Returns the [Entity](struct.Entity.html) ID associated with the given [EntityName](struct.EntityName.html).
    pub fn get_id(&self, name: &EntityName) -> Option<Entity> {
        self.to_id.get(name).copied()
    }

    /// Returns the [EntityName](struct.EntityName.html) associated with the given [Entity](struct.Entity.html) ID.
    pub fn get_name(&self, entity: Entity) -> Option<EntityName> {
        self.to_name.get(&entity).copied()
    }

    /// Canonizes a given [EntityName](struct.EntityName.html) and returns the associated [Entity](struct.Entity.html) ID.
    pub fn canonize_name(&mut self, name: &EntityName) -> Entity {
        match self.to_id.entry(*name) {
            Entry::Occupied(occupied) => *occupied.get(),
            Entry::Vacant(vacant) => {
                let entity = self.allocator.next().unwrap();
                vacant.insert(entity);
                self.to_name.insert(entity, *name);
                entity
            }
        }
    }

    /// Canonizes a given [Entity](struct.Entity.html) ID and returns the associated [EntityName](struct.EntityName.html).
    pub fn canonize_id(&mut self, entity: Entity) -> EntityName {
        match self.to_name.entry(entity) {
            Entry::Occupied(occupied) => *occupied.get(),
            Entry::Vacant(vacant) => {
                let uuid = Uuid::new_v4();
                let name = *uuid.as_bytes();
                vacant.insert(name);
                self.to_id.insert(name, entity);
                name
            }
        }
    }

    /// Canonizes the given entity and name pair.
    pub fn canonize(&mut self, entity: Entity, name: EntityName) -> Result<(), CanonizeError> {
        if let Some(existing) = self.to_id.get(&name) {
            if existing != &entity {
                return Err(CanonizeError::NameAlreadyBound(*existing, name));
            }
        }
        match self.to_name.entry(entity) {
            Entry::Occupied(occupied) => {
                if occupied.get() == &name {
                    Ok(())
                } else {
                    Err(CanonizeError::EntityAlreadyBound(entity, *occupied.get()))
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(name);
                self.to_id.insert(name, entity);
                Ok(())
            }
        }
    }
}

impl CustomEntitySerializer for Canon {
    type SerializedID = uuid::Uuid;
    /// Constructs the serializable representation of `Entity`
    fn to_serialized(&mut self, entity: Entity) -> Self::SerializedID {
        uuid::Uuid::from_bytes(self.canonize_id(entity))
    }

    /// Convert a `SerializedEntity` to an `Entity`.
    fn from_serialized(&mut self, serialized: Self::SerializedID) -> Entity {
        self.canonize_name(serialized.as_bytes())
    }
}
