use crate::{
    entity::{EntityAllocator, Entity},
    storage::{
        ArchetypeData, ArchetypeDescription, ComponentMeta, ComponentResourceSet,
        ComponentStorage, ComponentTypeId, TagMeta, TagStorage, TagTypeId,
    },
    world::World,
};
use serde::{de::DeserializeSeed, Deserializer};

pub struct WorldDeserializable<'a, 'b, CS: WorldDeserializer> {
    user: &'b CS,
    world: &'a World,
}

pub fn deserializable_world<'a, 'b, CS: WorldDeserializer>(
    world: &'a World,
    deserialize_impl: &'b CS,
) -> WorldDeserializable<'a, 'b, CS> {
    WorldDeserializable {
        world,
        user: deserialize_impl,
    }
}

pub trait WorldDeserializer {
    fn deserialize_archetype_description<D: for<'de> Deserializer<'de>>(
        &self,
        deserializer: D,
    ) -> Result<ArchetypeDescription, <D as Deserializer>::Error>;
    fn deserialize_components<D: for<'de> Deserializer<'de>>(
        &self,
        deserializer: D,
        component_type: &ComponentTypeId,
        component_meta: &ComponentMeta,
        components: &mut ComponentResourceSet,
    ) -> Result<(), <D as Deserializer>::Error>;
    fn deserialize_tags<D: for<'de> Deserializer<'de>>(
        &self,
        deserializer: D,
        tag_type: &TagTypeId,
        tag_meta: &TagMeta,
        tags: &mut TagStorage,
    ) -> Result<(), <D as Deserializer>::Error>;
    fn deserialize_entities<D: for<'de> Deserializer<'de>>(
        &self,
        deserializer: D,
        entity_allocator: &mut EntityAllocator,
        entities: &mut Vec<Entity>,
    ) -> Result<(), <D as Deserializer>::Error>;
}


struct WorldDeserialize<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a World,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for WorldDeserialize<'a, 'b, WD> {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(serde::de::IgnoredAny);
        Ok(())
    }
}


