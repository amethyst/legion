//! Serde (de)serialization of worlds.
//!
//! As component types are not known at compile time, the world must be provided with the
//! means to serialize each component. This is provided by the [`WorldSerializer`] implementation.
//! This implementation also describes how [`ComponentTypeId`](super::storage::ComponentTypeId)s
//! (which are not stable between compiles) are mapped to stable type identifiers. Components
//! that are not known to the serializer will be omitted from the serialized output.
//!
//! The [`Registry`] provides a [`WorldSerializer`] implementation suitable for most situations.
//!
//! Serializing all entities with a `Position` component to JSON.
//! ```
//! # use legion::*;
//! # use legion::serialize::Canon;
//! # let world = World::default();
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct Position;
//! // create a registry which uses strings as the external type ID
//! let mut registry = Registry::<String>::default();
//! registry.register::<Position>("position".to_string());
//! registry.register::<f32>("f32".to_string());
//! registry.register::<bool>("bool".to_string());
//!
//! // serialize entities with the `Position` component
//! let entity_serializer = Canon::default();
//! let json = serde_json::to_value(&world.as_serializable(
//!     component::<Position>(),
//!     &registry,
//!     &entity_serializer,
//! ))
//! .unwrap();
//! println!("{:#}", json);
//!
//! // registries are also serde deserializers
//! use serde::de::DeserializeSeed;
//! let world: World = registry
//!     .as_deserialize(&entity_serializer)
//!     .deserialize(json)
//!     .unwrap();
//! ```
//!
//! # `Box<dyn LayoutFilter + Send>` as a layout filter
//! [`World::as_serializable()`](super::World::as_serializable) accepts an implementor of the
//! [`LayoutFilter`](super::query::LayoutFilter) trait that is, in particular, also implemented
//! for `Box<dyn LayoutFilter + Send>`. This allows to store filter objects of different
//! types inside [`Box`](std::boxed::Box)es in some collection and process them all together:
//! ```
//! # use legion::*;
//! # use legion::query::LayoutFilter;
//! # use legion::serialize::Canon;
//! # let world = World::default();
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct Name;
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct Position;
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct Velocity;
//! // setup world serializer
//! let mut registry = Registry::<String>::default();
//! registry.register::<Name>("name".to_string());
//! registry.register::<Position>("position".to_string());
//! registry.register::<Velocity>("velocity".to_string());
//! let entity_serializer = Canon::default();
//!
//! // store serialization requests with different entity filters together
//! let serialize_requests: Vec<(&str, Box<dyn LayoutFilter + Send>)> = vec![
//!     ("all entities", Box::new(any())),
//!     ("having names", Box::new(component::<Name>())),
//!     ("without velocity", Box::new(!component::<Velocity>())),
//! ];
//!
//! // process all requests
//! for (title, filter) in serialize_requests {
//!     let json =
//!         serde_json::to_value(&world.as_serializable(filter, &registry, &entity_serializer))
//!             .unwrap();
//!     println!("{}: {:#}", title, json);
//! }
//! ```
//!
//! ## Why also `Send`?
//! [`Send`](std::marker::Send) requirement is needed to allow to store filters in
//! [`Resources`](super::Resources) accessed by non thread-local systems. This allows e.g.
//! to implement serialization requests sending from different systems and then processing them
//! all together outside the [`Schedule`](super::Schedule) execution.

#[cfg(feature = "type-uuid")]
pub use crate::internals::serialize::SerializableTypeUuid;
pub use crate::internals::serialize::{
    de::WorldDeserializer,
    id::{set_entity_serializer, Canon, CustomEntitySerializer, EntityName, EntitySerializer},
    ser::{SerializableWorld, WorldSerializer},
    AutoTypeKey, DeserializeIntoWorld, DeserializeNewWorld, Registry, TypeKey, UnknownType,
};
