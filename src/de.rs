use crate::{
    entity::{EntityAllocator, Entity},
    storage::{
        ArchetypeData, ArchetypeDescription, ComponentMeta, ComponentResourceSet,
        ComponentStorage, ComponentTypeId, TagMeta, TagStorage, TagTypeId,
    },
    world::World,
};
use serde::{self, de::{self, DeserializeSeed, Visitor}, Deserializer, Deserialize};
use std::cell::RefCell;

pub fn deserialize<'dd, 'a, 'b, CS: WorldDeserializer, D: Deserializer<'dd>>(
    world: &'a mut World,
    deserialize_impl: &'b CS,
    deserializer: D,
) -> Result<(), <D as Deserializer<'dd>>::Error> {
    let world_refcell = RefCell::new(world);
    let deserializable = WorldDeserialize {
        world: &world_refcell,
        user: deserialize_impl,
    };
    <WorldDeserialize<CS> as DeserializeSeed>::deserialize(deserializable, deserializer)
}

pub trait WorldDeserializer {
    fn deserialize_archetype_description<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
    ) -> Result<ArchetypeDescription, <D as Deserializer<'de>>::Error>;
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
    world: &'a RefCell<&'a mut World>,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for WorldDeserialize<'a, 'b, WD> {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(SeqDeserializer(ArchetypeDeserializer {
            user: self.user,
            world: self.world,
        }))?;
        Ok(())
    }
}
#[derive(Deserialize, Debug)]
#[serde(field_identifier, rename_all = "lowercase")]
enum ArchetypeField {
    Description,
    Tags,
    ChunkSets,
}
struct ArchetypeDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a RefCell<&'a mut World>,
}
impl<'a, 'b, WD: WorldDeserializer> Clone for ArchetypeDeserializer<'a, 'b, WD> {
    fn clone(&self) -> Self {
        Self {
            user: self.user,
            world: self.world,
        }
    }
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for ArchetypeDeserializer<'a, 'b, WD> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        impl<'a, 'b, 'de, WD: WorldDeserializer> Visitor<'de> for ArchetypeDeserializer<'a, 'b, WD> {
            type Value = ();

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Archetype")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut archetype_idx = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        ArchetypeField::Description => {
                            println!("desc");
                            archetype_idx = Some(map.next_value_seed(ArchetypeDescriptionDeserialize {
                                user: self.user,
                                world: self.world,
                            })?);
                        }
                        ArchetypeField::Tags => {
                            println!("tags");
                            let archetype_idx = archetype_idx.expect("expected archetype description before tags");
                            let mut world = self.world.borrow_mut();
                            let archetype_data = &mut world.storage_mut().archetypes_mut()[archetype_idx];
                        }
                        ArchetypeField::ChunkSets => {
                            println!("chunk_set");
                            let archetype_idx = archetype_idx.expect("expected archetype description before tags");
                            let mut world = self.world.borrow_mut();
                            let archetype_data = &mut world.storage_mut().archetypes_mut()[archetype_idx];

                        }
                    }
                }
                Err(de::Error::missing_field("data"))
            }
        }
        println!("deserialize struct");
        const FIELDS: &'static [&'static str] = &["description", "tags", "chunk_sets"];
        deserializer.deserialize_struct("Archetype", FIELDS, self)
    }
}

pub struct SeqDeserializer<T>(T);

impl<'de, T: DeserializeSeed<'de> + Clone> DeserializeSeed<'de> for SeqDeserializer<T> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}
impl<'de, T: DeserializeSeed<'de> + Clone> Visitor<'de> for SeqDeserializer<T> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("sequence of objects")
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        while let Some(_) = seq.next_element_seed::<T>(self.0.clone())? {}
        Ok(())
    }
}
struct ArchetypeDescriptionDeserialize<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a RefCell<&'a mut World>,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for ArchetypeDescriptionDeserialize<'a, 'b, WD> {
    type Value = usize;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        let archetype_desc = <WD as WorldDeserializer>::deserialize_archetype_description::<D>(
            self.user,
            deserializer
        )?;
        let mut world = self.world.borrow_mut();
        let mut storage = world.storage_mut();
        Ok(storage.archetypes().iter().position(|a| a.description() == &archetype_desc).unwrap_or_else(||{
            println!(" alloc archetype");
            let (idx, _) = storage.alloc_archetype(archetype_desc);
            idx
        }))
    }
}

struct TagsDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a RefCell<&'a mut World>,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for ArchetypeDescriptionDeserialize<'a, 'b, WD> {
    type Value = usize;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        let archetype_desc = <WD as WorldDeserializer>::deserialize_archetype_description::<D>(
            self.user,
            deserializer
        )?;
        let mut world = self.world.borrow_mut();
        let mut storage = world.storage_mut();
        Ok(storage.archetypes().iter().position(|a| a.description() == &archetype_desc).unwrap_or_else(||{
            println!(" alloc archetype");
            let (idx, _) = storage.alloc_archetype(archetype_desc);
            idx
        }))
    }
}
