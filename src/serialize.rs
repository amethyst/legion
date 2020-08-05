//! Serde (de)serialization of worlds.
//!
//! As component types are not known at compile time, the world must be provided with the
//! means to serialize each component. This is provided by the
//! [WorldSerializer](trait.WorldSerializer.html) implementation. This implementation
//! also describes how [ComponentTypeIDs](../storage/struct.ComponentTypeId.html) (which
//! are not stable between compiles) are mapped to stable type identifiers. Components that are
//! not known to the serializer will be omitted from the serialized output.
//!
//! The [Registry](struct.Registry.html) provides a
//! [WorldSerializer](trait.WorldSerializer.html) implementation suitable for most
//! situations.
//!
//! Serializing all entities with a `Position` component to JSON.
//! ```
//! # use legion::*;
//! # let world = World::default();
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct Position;
//! // create a registry which uses strings as the external type ID
//! let mut registry = Registry::<String>::new();
//! registry.register::<Position>("position".to_string());
//! registry.register::<f32>("f32".to_string());
//! registry.register::<bool>("bool".to_string());
//!
//! // serialize entities with the `Position` component
//! let json = serde_json::to_value(&world.as_serializable(component::<Position>(), &registry)).unwrap();
//! println!("{:#}", json);
//!
//! // registries are also serde deserializers
//! use serde::de::DeserializeSeed;
//! let world: World = registry.as_deserialize().deserialize(json).unwrap();
//! ```

pub use crate::internals::serialize::{
    de::WorldDeserializer,
    id::{Canon, EntityName},
    ser::{SerializableWorld, WorldSerializer},
    AutoTypeKey, CanonSource, DeserializeIntoWorld, DeserializeNewWorld, Registry, TypeKey,
};

#[cfg(feature = "type-uuid")]
pub use crate::internals::serialize::SerializableTypeUuid;
