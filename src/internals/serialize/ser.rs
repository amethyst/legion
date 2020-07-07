//! World serialization types.

use super::{
    entities::ser::EntitiesLayoutSerializer, packed::ser::PackedLayoutSerializer, WorldField,
    WorldMeta,
};
use crate::internals::{query::filter::LayoutFilter, storage::ComponentTypeId, world::World};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// Describes a type which knows how to deserialize the components in a world.
pub trait WorldSerializer {
    /// The stable type ID used to identify each component type.
    type TypeId: Serialize + Ord;

    /// Converts a runtime component type ID into the serialized type ID.
    fn map_id(&self, type_id: ComponentTypeId) -> Option<Self::TypeId>;

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
}

/// A serializable representation of a world.
pub struct SerializableWorld<'a, F: LayoutFilter, W: WorldSerializer> {
    world: &'a World,
    filter: F,
    world_serializer: &'a W,
}

impl<'a, F: LayoutFilter, W: WorldSerializer> SerializableWorld<'a, F, W> {
    pub(crate) fn new(world: &'a World, filter: F, world_serializer: &'a W) -> Self {
        Self {
            world,
            filter,
            world_serializer,
        }
    }
}

impl<'a, F: LayoutFilter, W: WorldSerializer> Serialize for SerializableWorld<'a, F, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_world(serializer, self.world, &self.filter, self.world_serializer)
    }
}

fn serialize_world<S, F, W>(
    serializer: S,
    world: &World,
    filter: &F,
    world_serializer: &W,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    F: LayoutFilter,
    W: WorldSerializer,
{
    let human_readable = serializer.is_human_readable();
    let mut root = serializer.serialize_map(Some(2))?;

    // serialize world metadata
    root.serialize_entry(
        &WorldField::_Meta,
        &WorldMeta {
            entity_id_stride: world.entity_allocator().stride(),
            entity_id_offset: world.entity_allocator().offset(),
            entity_id_next: world.entity_allocator().head(),
            component_groups: world
                .groups()
                .iter()
                .filter(|group| group.components().count() > 1)
                .map(|group| {
                    group
                        .components()
                        .filter_map(|type_id| world_serializer.map_id(type_id))
                        .collect()
                })
                .collect(),
        },
    )?;

    if human_readable {
        // serialize per-entity representation
        root.serialize_entry(
            &WorldField::Entities,
            &EntitiesLayoutSerializer {
                world_serializer,
                world,
                filter,
            },
        )?;
    } else {
        // serialize machine-optimised representation
        root.serialize_entry(
            &WorldField::Packed,
            &PackedLayoutSerializer {
                world_serializer,
                world,
                filter,
            },
        )?;
    }

    root.end()
}
