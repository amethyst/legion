//! World serialization types.

use super::{
    entities::ser::EntitiesLayoutSerializer, packed::ser::PackedLayoutSerializer, WorldField,
};
use crate::internals::{
    query::filter::LayoutFilter, storage::component::ComponentTypeId, world::World,
};
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
    let mut root = serializer.serialize_map(Some(1))?;

    crate::internals::entity::serde::UNIVERSE.with(|cell| {
        *cell.borrow_mut() = Some(world.universe().clone());

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

        *cell.borrow_mut() = None;
        root.end()
    })
}
