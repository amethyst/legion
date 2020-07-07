//! World deserialization types.

use super::{
    entities::de::EntitiesLayoutDeserializer, packed::de::PackedLayoutDeserializer, WorldField,
    WorldMeta,
};
use crate::{
    storage::{ArchetypeIndex, ComponentTypeId, EntityLayout, GroupDef, UnknownComponentStorage},
    world::{Universe, World, WorldOptions},
};
use serde::{
    de::{DeserializeSeed, MapAccess, Visitor},
    Deserialize, Deserializer,
};

/// Describes a type which knows how to deserialize the components in a world.
pub trait WorldDeserializer {
    /// The stable type ID used to identify each component type.
    type TypeId: for<'de> Deserialize<'de>;

    /// Converts the serialized type ID into a runtime component type ID.
    fn unmap_id(&self, type_id: &Self::TypeId) -> Option<ComponentTypeId>;

    /// Adds the specified component to the given entity layout.
    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout);

    /// Deserializes a slice of components and inserts them into the given storage.
    fn deserialize_component_slice<'de, D: Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        storage: &mut dyn UnknownComponentStorage,
        arch_index: ArchetypeIndex,
        deserializer: D,
    ) -> Result<(), D::Error>;

    /// Deserializes a single component and returns it as a boxed u8 slice.
    fn deserialize_component<'de, D: Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        deserializer: D,
    ) -> Result<Box<[u8]>, D::Error>;
}

pub(crate) struct Wrapper<T: WorldDeserializer>(pub T);

impl<'de, W: WorldDeserializer> DeserializeSeed<'de> for Wrapper<W> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(WorldVisitor {
            world_deserializer: self.0,
        })
    }
}

struct WorldVisitor<W: WorldDeserializer> {
    world_deserializer: W,
}

impl<'de, W: WorldDeserializer> Visitor<'de> for WorldVisitor<W> {
    type Value = World;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("map")
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut world = None;

        while let Some(key) = map.next_key()? {
            match key {
                WorldField::_Meta => {
                    if world.is_some() {
                        return Err(serde::de::Error::duplicate_field("_meta"));
                    }
                    world = Some(map.next_value_seed(MetaDeserializer {
                        world_deserializer: &self.world_deserializer,
                    })?);
                }
                WorldField::Packed => {
                    if let Some(world) = &mut world {
                        map.next_value_seed(PackedLayoutDeserializer {
                            world_deserializer: &self.world_deserializer,
                            world,
                        })?;
                    } else {
                        return Err(serde::de::Error::missing_field("_meta"));
                    }
                }
                WorldField::Entities => {
                    if let Some(world) = &mut world {
                        map.next_value_seed(EntitiesLayoutDeserializer {
                            world_deserializer: &self.world_deserializer,
                            world,
                        })?;
                    } else {
                        return Err(serde::de::Error::missing_field("_meta"));
                    }
                }
            }
        }

        let world = world.ok_or_else(|| serde::de::Error::missing_field("_meta"))?;
        Ok(world)
    }
}

struct MetaDeserializer<'a, W: WorldDeserializer> {
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for MetaDeserializer<'a, W> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let meta = WorldMeta::<W::TypeId>::deserialize(deserializer)?;
        let universe = Universe::sharded(meta.entity_id_offset, meta.entity_id_stride);
        universe.entity_allocator().skip(meta.entity_id_next);
        let options = WorldOptions {
            groups: meta
                .component_groups
                .iter()
                .map(|group| {
                    GroupDef::from_vec(
                        group
                            .iter()
                            .filter_map(|id| self.world_deserializer.unmap_id(id))
                            .collect(),
                    )
                })
                .collect(),
        };

        Ok(universe.create_world_with_options(options))
    }
}
