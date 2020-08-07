use crate::{internals::entity::EntityHasher, world::Allocate, Entity};
use serde::{Deserialize, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashMap};
use thiserror::Error;
use uuid::Uuid;

/// Describes how to serialize and deserialize a runtime `Entity` ID.
pub trait EntitySerializer {
    /// Serializes an `Entity` by constructing the serializable representation
    /// and passing it into `serialize_fn`.
    fn serialize(
        &mut self,
        entity: Entity,
        serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize),
    );

    /// Deserializes an `Entity`.
    fn deserialize(
        &mut self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error>;
}

thread_local! {
    static SERIALIZER: RefCell<Box<dyn EntitySerializer>> = RefCell::new(Box::new(Canon::default()));
}

/// Runs the provided function with this canon as context for Entity (de)serialization.
pub fn run_as_context<F: FnOnce() -> R, R>(context: &mut Box<dyn EntitySerializer>, f: F) -> R {
    SERIALIZER.with(|cell| {
        // swap context into TLS
        {
            let mut existing = cell.borrow_mut();
            std::mem::swap(context, &mut *existing);
        }

        let result = (f)();

        // swap context back out of LTS
        {
            let mut existing = cell.borrow_mut();
            std::mem::swap(context, &mut *existing);
        }

        result
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
            entity_serializer.serialize(*self, &mut move |serializable| {
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
            let mut entity_serializer = cell.borrow_mut();
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            entity_serializer
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

impl EntitySerializer for Canon {
    fn serialize(
        &mut self,
        entity: Entity,
        serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize),
    ) {
        let name = Uuid::from_bytes(self.canonize_id(entity));
        (serialize_fn)(&name);
    }

    fn deserialize(
        &mut self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error> {
        let name = erased_serde::deserialize::<Uuid>(deserializer)?;
        let entity = self.canonize_name(name.as_bytes());
        Ok(entity)
    }
}
