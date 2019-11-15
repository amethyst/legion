use crate::{
    entity::{Entity, EntityAllocator},
    storage::{
        ArchetypeData, ArchetypeDescription, ComponentMeta, ComponentResourceSet, ComponentStorage,
        ComponentTypeId, TagMeta, TagStorage, TagTypeId, Tags,
    },
    world::World,
};
use serde::{
    self,
    de::{self, DeserializeSeed, Visitor},
    Deserialize, Deserializer,
};
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
    fn deserialize_components<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        component_type: &ComponentTypeId,
        component_meta: &ComponentMeta,
        components: &mut ComponentResourceSet,
    ) -> Result<(), <D as Deserializer<'de>>::Error>;
    fn deserialize_tags<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        tag_type: &TagTypeId,
        tag_meta: &TagMeta,
        tags: &mut TagStorage,
    ) -> Result<(), <D as Deserializer<'de>>::Error>;
    fn deserialize_entities<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        entity_allocator: &mut EntityAllocator,
        entities: &mut Vec<Entity>,
    ) -> Result<(), <D as Deserializer<'de>>::Error>;
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
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de>
    for ArchetypeDeserializer<'a, 'b, WD>
{
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
                            archetype_idx =
                                Some(map.next_value_seed(ArchetypeDescriptionDeserialize {
                                    user: self.user,
                                    world: self.world,
                                })?);
                        }
                        ArchetypeField::Tags => {
                            println!("tags");
                            let archetype_idx =
                                archetype_idx.expect("expected archetype description before tags");
                            let mut world = self.world.borrow_mut();
                            let archetype_data =
                                &mut world.storage_mut().archetypes_mut()[archetype_idx];
                            map.next_value_seed(TagsDeserializer {
                                user: self.user,
                                archetype: archetype_data,
                            })?;
                        }
                        ArchetypeField::ChunkSets => {
                            println!("chunk_set");
                            let archetype_idx =
                                archetype_idx.expect("expected archetype description before tags");
                            let mut world = self.world.borrow_mut();
                            map.next_value_seed(ChunkSetDeserializer {
                                user: self.user,
                                world: &mut *world,
                                archetype_idx,
                            })?;
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
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de>
    for ArchetypeDescriptionDeserialize<'a, 'b, WD>
{
    type Value = usize;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        let archetype_desc = <WD as WorldDeserializer>::deserialize_archetype_description::<D>(
            self.user,
            deserializer,
        )?;
        let mut world = self.world.borrow_mut();
        let mut storage = world.storage_mut();
        Ok(storage
            .archetypes()
            .iter()
            .position(|a| a.description() == &archetype_desc)
            .unwrap_or_else(|| {
                println!(" alloc archetype");
                let (idx, _) = storage.alloc_archetype(archetype_desc);
                idx
            }))
    }
}

struct TagsDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    archetype: &'a mut ArchetypeData,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for TagsDeserializer<'a, 'b, WD> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        println!("wut");
        deserializer.deserialize_seq(self)
    }
}

impl<'de, 'a, 'b, WD: WorldDeserializer> Visitor<'de> for TagsDeserializer<'a, 'b, WD> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("sequence of objects")
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        // TODO fix mutability issue in the storage API here?
        let tags = unsafe { &mut *(self.archetype.tags() as *const Tags as *mut Tags) };
        let tag_types = self.archetype.description().tags();
        let mut idx = 0;
        loop {
            let (tag_type, tag_meta) = tag_types[idx];
            let tag_storage = tags
                .get_mut(tag_type)
                .expect("tag storage not present when deserializing");
            if let None = seq.next_element_seed(TagStorageDeserializer {
                user: self.user,
                tag_storage,
                tag_type: &tag_type,
                tag_meta: &tag_meta,
            })? {
                break;
            }
            idx += 1;
        }
        Ok(())
    }
}

struct TagStorageDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    tag_storage: &'a mut TagStorage,
    tag_type: &'a TagTypeId,
    tag_meta: &'a TagMeta,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de>
    for TagStorageDeserializer<'a, 'b, WD>
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        println!("user deserialize tag {:?}", self.tag_type);
        self.user
            .deserialize_tags(deserializer, self.tag_type, self.tag_meta, self.tag_storage)?;
        Ok(())
    }
}

struct ChunkSetDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a mut World,
    archetype_idx: usize,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for ChunkSetDeserializer<'a, 'b, WD> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, 'a, 'b, WD: WorldDeserializer> Visitor<'de> for ChunkSetDeserializer<'a, 'b, WD> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("sequence of objects")
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        // TODO fix mutability issue in the storage API here?
        let archetype = self.world.storage().archetypes()[self.archetype_idx];
        let tags = unsafe { &mut *(archetype.tags() as *const Tags as *mut Tags) };
        let mut idx = 0;
        loop {
            let chunk_set = self.world.find_or_create_chunk(self.archetype_idx, tags);
            if let None = seq.next_element_seed(ChunkDeserializer {
                user: self.user,
                world: self.world,
                archetype_idx: self.archetype_idx,
                chunkset_idx: chunk_set,
            })? {
                break;
            }
            idx += 1;
        }
        Ok(())
    }
}

#[derive(Deserialize, Debug)]
#[serde(field_identifier, rename_all = "lowercase")]
enum ChunkField {
    Entities,
    Components,
}
struct ChunkDeserializer<'a, 'b, WD: WorldDeserializer> {
    user: &'b WD,
    world: &'a mut World,
    archetype_idx: usize,
    chunkset_idx: usize,
}
impl<'de, 'a, 'b, WD: WorldDeserializer> DeserializeSeed<'de> for ChunkDeserializer<'a, 'b, WD> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_struct("Chunk", &["entities", "components"], self)
    }
}

impl<'de, 'a, 'b, WD: WorldDeserializer> Visitor<'de> for ChunkDeserializer<'a, 'b, WD> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("struct Chunk")
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: de::MapAccess<'de>,
    {
        let mut chunk_idx = None;
        while let Some(key) = map.next_key()? {
            match key {
                ChunkField::Entities => {
                    println!("entities");
                    chunk_idx = Some(map.next_value_seed(EntitiesDeserializer {
                        user: self.user,
                        world: self.world,
                        archetype_idx: self.archetype_idx,
                        chunkset_idx: self.chunkset_idx,
                        chunk_idx: chunk_idx,
                    })?);
                }
                ChunkField::Components => {
                    chunk_idx = Some(map.next_value_seed(ComponentsDeserializer {
                        user: self.user,
                        world: self.world,
                        archetype_idx: self.archetype_idx,
                        chunkset_idx: self.chunkset_idx,
                        chunk_idx: chunk_idx,
                    })?);
                }
            }
        }
        // // TODO fix mutability issue in the storage API here?
        // let tags = unsafe { &mut *(self.archetype.tags() as *const Tags as *mut Tags) };
        // let mut idx = 0;
        // loop {
        //     let chunk_set = self.world.find_or_create_chunk(self.archetype_idx, tags);

        //     let (tag_type, tag_meta) = tag_types[idx];
        //     let tag_storage = tags
        //         .get_mut(tag_type)
        //         .expect("tag storage not present when deserializing");
        //     if let None = seq.next_element_seed(TagStorageDeserializer {
        //         user: self.user,
        //         tag_storage,
        //         tag_type: &tag_type,
        //         tag_meta: &tag_meta,
        //     })? {
        //         break;
        //     }
        //     idx += 1;
        // }
        Ok(())
    }
}
