//! World deserialization types.

use super::{
    entities::de::EntitiesLayoutDeserializer, packed::de::PackedLayoutDeserializer, WorldField,
};
use crate::internals::{
    storage::{
        archetype::{ArchetypeIndex, EntityLayout},
        component::ComponentTypeId,
        UnknownComponentStorage,
    },
    world::World,
};
use serde::{
    de::{MapAccess, Visitor},
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

pub struct WorldVisitor<'a, W: WorldDeserializer> {
    pub world_deserializer: &'a W,
    pub world: &'a mut World,
}

impl<'a, 'de, W: WorldDeserializer> Visitor<'de> for WorldVisitor<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("map")
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        crate::internals::entity::serde::UNIVERSE.with(|cell| {
            *cell.borrow_mut() = Some(self.world.universe().clone());

            while let Some(key) = map.next_key()? {
                match key {
                    WorldField::Packed => {
                        map.next_value_seed(PackedLayoutDeserializer {
                            world_deserializer: self.world_deserializer,
                            world: self.world,
                        })?;
                    }
                    WorldField::Entities => {
                        map.next_value_seed(EntitiesLayoutDeserializer {
                            world_deserializer: self.world_deserializer,
                            world: self.world,
                        })?;
                    }
                }
            }

            *cell.borrow_mut() = None;
            Ok(())
        })
    }
}
