use crate::{internals::entity::EntityHasher, world::Allocate, Entity};
use serde::{Deserialize, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashMap};
use thiserror::Error;
use uuid::Uuid;

thread_local! {
    static CANON: RefCell<Canon> = RefCell::new(Canon::default());
}

impl Serialize for Entity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CANON.with(|cell| {
            let mut canon = cell.borrow_mut();
            let name = Uuid::from_bytes(canon.canonize_id(*self));
            name.serialize(serializer)
        })
    }
}

impl<'de> Deserialize<'de> for Entity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        CANON.with(|cell| {
            let mut canon = cell.borrow_mut();
            let name = Uuid::deserialize(deserializer)?;
            let entity = canon.canonize_name(name.as_bytes());
            Ok(entity)
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
    /// Runs the provided function with this canon as context for Entity (de)serialization.
    pub fn run_as_context<F: FnOnce() -> R, R>(&mut self, f: F) -> R {
        CANON.with(|cell| {
            // swap self into TLS
            {
                let mut existing = cell.borrow_mut();
                std::mem::swap(self, &mut *existing);
            }

            let result = (f)();

            // swap self back out of LTS
            {
                let mut existing = cell.borrow_mut();
                std::mem::swap(self, &mut *existing);
            }

            result
        })
    }

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
