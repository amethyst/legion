use crate::{internals::entity::EntityHasher, world::Allocate, Entity};
use scoped_tls_hkt::scoped_thread_local;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{hash_map::Entry, HashMap};
use thiserror::Error;
use uuid::Uuid;

/// Describes how to serialize and deserialize a runtime `Entity` ID.
///
/// This trait is automatically implemented for types that implement [`CustomEntitySerializer`].
pub trait EntitySerializer: 'static {
    /// Serializes an `Entity` by constructing the serializable representation
    /// and passing it into `serialize_fn`.
    fn serialize(&self, entity: Entity, serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize));

    /// Deserializes an `Entity`.
    fn deserialize(
        &self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error>;
}

/// Describes a mapping between a runtime `Entity` ID and a serialized equivalent.
///
/// Implementing this trait will automatically implement [`EntitySerializer`] for the type.
///
/// Developers should be aware of their serialization/deserialization use-cases as well as
/// world-merge use cases when picking a `SerializedID`, as this type must identify unique entities
/// across world serialization/deserialization cycles as well as across world merges.
pub trait CustomEntitySerializer {
    /// The type used for serialized Entity IDs.
    type SerializedID: serde::Serialize + for<'a> serde::Deserialize<'a>;

    /// Constructs the serializable representation of `Entity`
    fn to_serialized(&self, entity: Entity) -> Self::SerializedID;

    /// Convert a `SerializedEntity` to an `Entity`.
    fn from_serialized(&self, serialized: Self::SerializedID) -> Entity;
}

impl<T> EntitySerializer for T
where
    T: CustomEntitySerializer + 'static,
{
    fn serialize(
        &self,
        entity: Entity,
        serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize),
    ) {
        let serialized = self.to_serialized(entity);
        serialize_fn(&serialized);
    }

    fn deserialize(
        &self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error> {
        let serialized =
            <<Self as CustomEntitySerializer>::SerializedID as serde::Deserialize>::deserialize(
                deserializer,
            )?;
        Ok(self.from_serialized(serialized))
    }
}

scoped_thread_local! {
    static ENTITY_SERIALIZER: dyn EntitySerializer
}

/// Sets the [`EntitySerializer`] currently being used to serialize or deserialize [`Entity`] IDs.
///
/// This is set automatically when serializing or deserializing a [`World`]. When serializing or
/// deserializing values outside a `World`, this needs to be set manually, passing in a reference
/// to the `EntitySerializer` and a closure that does the serializing/deserializing.
/// ```
/// # use legion::*;
/// # use legion::serialize::{Canon, set_entity_serializer};
/// # let mut world = World::default();
/// # #[derive(serde::Serialize, serde::Deserialize)]
/// # struct ContainsEntity(Entity);
/// # let contains_entity = ContainsEntity(world.push(()));
///
/// let entity_serializer = Canon::default();
/// set_entity_serializer(&entity_serializer, || {
///     serde_json::to_value(contains_entity).unwrap()
/// });
/// ```
pub fn set_entity_serializer<F, R>(entity_serializer: &dyn EntitySerializer, func: F) -> R
where
    F: FnOnce() -> R,
{
    ENTITY_SERIALIZER.set(entity_serializer, func)
}

impl Serialize for Entity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ENTITY_SERIALIZER.with(|entity_serializer| {
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
        ENTITY_SERIALIZER.with(|entity_serializer| {
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
    inner: parking_lot::RwLock<CanonInner>,
}

#[derive(Default, Debug)]
struct CanonInner {
    to_name: HashMap<Entity, EntityName, EntityHasher>,
    to_id: HashMap<EntityName, Entity>,
    allocator: Allocate,
}

impl Canon {
    /// Returns the [`Entity`] ID associated with the given [`EntityName`].
    pub fn get_id(&self, name: &EntityName) -> Option<Entity> {
        self.inner.read().to_id.get(name).copied()
    }

    /// Returns the [`EntityName`] associated with the given [`Entity`] ID.
    pub fn get_name(&self, entity: Entity) -> Option<EntityName> {
        self.inner.read().to_name.get(&entity).copied()
    }

    /// Canonizes a given [`EntityName`] and returns the associated [`Entity`] ID.
    pub fn canonize_name(&self, name: &EntityName) -> Entity {
        let mut inner = self.inner.write();
        let inner = &mut *inner;
        match inner.to_id.entry(*name) {
            Entry::Occupied(occupied) => *occupied.get(),
            Entry::Vacant(vacant) => {
                let entity = inner.allocator.next().unwrap();
                vacant.insert(entity);
                inner.to_name.insert(entity, *name);
                entity
            }
        }
    }

    /// Canonizes a given [`Entity`] ID and returns the associated [`EntityName`].
    pub fn canonize_id(&self, entity: Entity) -> EntityName {
        let mut inner = self.inner.write();
        match inner.to_name.entry(entity) {
            Entry::Occupied(occupied) => *occupied.get(),
            Entry::Vacant(vacant) => {
                let uuid = Uuid::new_v4();
                let name = *uuid.as_bytes();
                vacant.insert(name);
                inner.to_id.insert(name, entity);
                name
            }
        }
    }

    /// Canonizes the given entity and name pair.
    pub fn canonize(&self, entity: Entity, name: EntityName) -> Result<(), CanonizeError> {
        let mut inner = self.inner.write();
        if let Some(existing) = inner.to_id.get(&name) {
            if existing != &entity {
                return Err(CanonizeError::NameAlreadyBound(*existing, name));
            }
        }
        match inner.to_name.entry(entity) {
            Entry::Occupied(occupied) => {
                if occupied.get() == &name {
                    Ok(())
                } else {
                    Err(CanonizeError::EntityAlreadyBound(entity, *occupied.get()))
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(name);
                inner.to_id.insert(name, entity);
                Ok(())
            }
        }
    }
}

impl CustomEntitySerializer for Canon {
    type SerializedID = uuid::Uuid;

    fn to_serialized(&self, entity: Entity) -> Self::SerializedID {
        uuid::Uuid::from_bytes(self.canonize_id(entity))
    }

    fn from_serialized(&self, serialized: Self::SerializedID) -> Entity {
        self.canonize_name(serialized.as_bytes())
    }
}
