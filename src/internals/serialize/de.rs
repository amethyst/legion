//! World deserialization types.

use super::{
    entities::de::EntitiesLayoutDeserializer, id::run_as_context,
    packed::de::PackedLayoutDeserializer, EntitySerializerSource, UnknownType, WorldField,
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
pub trait WorldDeserializer: EntitySerializerSource {
    /// The stable type ID used to identify each component type in the serialized data.
    type TypeId: for<'de> Deserialize<'de>;

    /// Converts the serialized type ID into a runtime component type ID.
    fn unmap_id(&self, type_id: &Self::TypeId) -> Result<ComponentTypeId, UnknownType>;

    /// Adds the specified component to the given entity layout.
    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout);

    /// Deserializes a single component and inserts it into the given storage.
    fn deserialize_insert_component<'de, D: Deserializer<'de>>(
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

    fn visit_map<V>(self, map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        fn run<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>>(
            world_deserializer: &'a W,
            world: &'a mut World,
            mut map: V,
        ) -> Result<(), V::Error> {
            while let Some(key) = map.next_key()? {
                match key {
                    WorldField::Packed => {
                        map.next_value_seed(PackedLayoutDeserializer {
                            world_deserializer,
                            world,
                        })?;
                    }
                    WorldField::Entities => {
                        map.next_value_seed(EntitiesLayoutDeserializer {
                            world_deserializer,
                            world,
                        })?;
                    }
                }
            }
            Ok(())
        }

        if let Some(canon) = self.world_deserializer.entity_serializer() {
            let mut canon = canon.lock();
            run_as_context(&mut canon, move || {
                run(self.world_deserializer, self.world, map)
            })
        } else {
            run(self.world_deserializer, self.world, map)
        }
    }
}
