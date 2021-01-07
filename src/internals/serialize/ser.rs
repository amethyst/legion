//! World serialization types.

use super::{
    archetypes::ser::ArchetypeLayoutSerializer, entities::ser::EntitiesLayoutSerializer,
    id::run_as_context, EntitySerializer, UnknownType, WorldField,
};
use crate::{
    internals::{query::filter::LayoutFilter, storage::component::ComponentTypeId, world::World},
    storage::{ArchetypeIndex, UnknownComponentStorage},
};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// Describes a type which knows how to deserialize the components in a world.
pub trait WorldSerializer {
    /// The stable type ID used to identify each component type in the serialized data.
    type TypeId: Serialize + Ord;

    /// Converts a runtime component type ID into the serialized type ID.
    fn map_id(&self, type_id: ComponentTypeId) -> Result<Self::TypeId, UnknownType>;

    /// Serializes a single component.
    ///
    /// # Safety
    /// The pointer must point to a valid instance of the component type represented by
    /// the given component type ID.
    unsafe fn serialize_component<S: Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        serializer: S,
    ) -> Result<S::Ok, S::Error>;

    /// Serializes a slice of components.
    ///
    /// # Safety
    /// The pointer must point to a valid instance of the component type represented by
    /// the given component type ID.
    unsafe fn serialize_component_slice<S: Serializer>(
        &self,
        ty: ComponentTypeId,
        storage: &dyn UnknownComponentStorage,
        archetype: ArchetypeIndex,
        serializer: S,
    ) -> Result<S::Ok, S::Error>;
}

/// A serializable representation of a world.
pub struct SerializableWorld<'a, F: LayoutFilter, W: WorldSerializer, E: EntitySerializer> {
    world: &'a World,
    filter: F,
    world_serializer: &'a W,
    entity_serializer: &'a E,
}

impl<'a, F: LayoutFilter, W: WorldSerializer, E: EntitySerializer> SerializableWorld<'a, F, W, E> {
    pub(crate) fn new(
        world: &'a World,
        filter: F,
        world_serializer: &'a W,
        entity_serializer: &'a E,
    ) -> Self {
        Self {
            world,
            filter,
            world_serializer,
            entity_serializer,
        }
    }
}

impl<'a, F: LayoutFilter, W: WorldSerializer, E: EntitySerializer> Serialize
    for SerializableWorld<'a, F, W, E>
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_world(
            serializer,
            self.world,
            &self.filter,
            self.world_serializer,
            self.entity_serializer,
        )
    }
}

fn serialize_world<S, F, W, E>(
    serializer: S,
    world: &World,
    filter: &F,
    world_serializer: &W,
    entity_serializer: &E,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    F: LayoutFilter,
    W: WorldSerializer,
    E: EntitySerializer,
{
    let human_readable = serializer.is_human_readable();
    let mut root = serializer.serialize_map(Some(1))?;

    let mut hoist = core::cell::Cell::new(None);
    let hoist_ref = &mut hoist;
    let root_ref = &mut root;

    run_as_context(entity_serializer, || {
        let result = if human_readable {
            // serialize per-entity representation
            root_ref.serialize_entry(
                &WorldField::Entities,
                &EntitiesLayoutSerializer {
                    world_serializer,
                    world,
                    filter,
                },
            )
        } else {
            // serialize machine-optimised representation
            root_ref.serialize_entry(
                &WorldField::Packed,
                &ArchetypeLayoutSerializer {
                    world_serializer,
                    world,
                    filter,
                },
            )
        };
        hoist_ref.set(Some(result));
    });

    hoist.into_inner().unwrap()?;
    root.end()
}
