//! World deserialization types.

use super::{
    archetypes::de::ArchetypeLayoutDeserializer, entities::de::EntitiesLayoutDeserializer,
    id::run_as_context, EntitySerializer, UnknownType, WorldField,
};
use crate::{
    internals::{
        storage::{archetype::EntityLayout, component::ComponentTypeId},
        world::World,
    },
    storage::UnknownComponentWriter,
};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};

/// Describes a type which knows how to deserialize the components in a world.
pub trait WorldDeserializer {
    /// The stable type ID used to identify each component type in the serialized data.
    type TypeId: for<'de> Deserialize<'de>;

    /// Converts the serialized type ID into a runtime component type ID.
    fn unmap_id(&self, type_id: &Self::TypeId) -> Result<ComponentTypeId, UnknownType>;

    /// Adds the specified component to the given entity layout.
    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout);

    /// Deserializes a slice of components and inserts them into the given storage.
    fn deserialize_component_slice<'a, 'de, D: Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        storage: UnknownComponentWriter<'a>,
        deserializer: D,
    ) -> Result<(), D::Error>;

    /// Deserializes a single component and returns it as a boxed u8 slice.
    fn deserialize_component<'de, D: Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        deserializer: D,
    ) -> Result<Box<[u8]>, D::Error>;

    /// Calls `callback` with the Entity ID serializer
    fn with_entity_serializer(&self, callback: &mut dyn FnMut(&dyn EntitySerializer));
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
                        map.next_value_seed(ArchetypeLayoutDeserializer {
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

        let mut hoist = core::cell::Cell::new(None);
        let hoist_ref = &mut hoist;
        // since it's a double closure, and one is FnMut and the inner one is FnOnce,
        // we need to do some ugly hoisting
        let mut world = self.world;
        let mut map_hoist = Some(map);
        let world_deserializer = self.world_deserializer;
        world_deserializer.with_entity_serializer(&mut |canon| {
            let world_inner = &mut world;
            let hoist_ref_inner = &hoist_ref;
            run_as_context(canon, || {
                let map = map_hoist.take().unwrap();
                hoist_ref_inner.set(Some(run(world_deserializer, *world_inner, map)));
            })
        });
        hoist.into_inner().unwrap()
    }
}
