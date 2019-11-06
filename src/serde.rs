use crate::{
    entity::Entity,
    storage::{
        ArchetypeData, ArchetypeDescription, Chunkset, ComponentResourceSet, ComponentTypeId,
        TagMeta, TagStorage, TagTypeId,
        ComponentMeta,
        ComponentStorage,
    },
    world::World,
};
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};

pub struct WorldSerializable<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    world: &'a World,
}

pub fn serializable_world<'a, 'b, CS: WorldSerializer>(world: &'a World, serialize_impl: &'b CS) -> WorldSerializable<'a, 'b, CS> {
    WorldSerializable {
        world,
        world_serializer: serialize_impl,
    }
}


/*
// Structure optimized for saving and loading:
[
    (
        // Description of archetype
        archetype: {},
        // Tag data arrays. One inner array per chunk set. Indices match chunk set indices
        tags: [
            // Tag values. One element per chunk set. Indices match chunk set indices
            [TAG_DATA]
        ],
        chunksets: [
            // CHUNK SET. One array element per array of chunks in the chunkset
            [
                // CHUNK
                (
                    // ENTITIES in the chunk
                    entities: [Entity],
                    // COMPONENT STORAGE: One array per component type, as per the archetype.
                    // Component type indices in archetype correspond to indices here
                    components: [
                        // COMPONENT RESOURCE SET: The actual component data. One element per entity
                        [COMPONENT_DATA],
                        ...
                    ],
                ),
                ...
            ],
            ...
        ],
    ),
    ...

]
*/

pub trait WorldSerializer {
    fn can_serialize(
        &self,
        archetype_desc: &ArchetypeDescription,
    ) -> bool;
    fn serialize_archetype_description<S: Serializer>(
        &self,
        serializer: S,
        archetype_desc: &ArchetypeDescription,
    ) -> Result<S::Ok, S::Error>;
    fn serialize_components<S: Serializer>(
        &self,
        serializer: S,
        component_type: &ComponentTypeId,
        component_meta: &ComponentMeta,
        components: &ComponentResourceSet,
    ) -> Result<S::Ok, S::Error>;
    fn serialize_tags<S: Serializer>(
        &self,
        serializer: S,
        tag_type: &TagTypeId,
        tag_meta: &TagMeta,
        tags: &TagStorage,
    ) -> Result<S::Ok, S::Error>;
    fn serialize_entities<S: Serializer>(
        &self,
        serializer: S,
        entities: &[Entity]
    ) -> Result<S::Ok, S::Error>;
}

impl<'a, 'b, CS: WorldSerializer> Serialize for WorldSerializable<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let storage = self.world.storage();
        serializer.collect_seq(
            storage
                .archetypes()
                .iter()
                .filter(|archetype| self.world_serializer.can_serialize(archetype.description()))
                .map(|archetype| ArchetypeSerializer {
                    world_serializer: self.world_serializer,
                    archetype,
                }),
        )
        // let storage = self.world.storage();
        // let mut tag_slice_buffer: Vec<(&TagTypeId, &TagStorage)> = Vec::new();
        // for archetype in storage.archetypes() {
        //     let desc = archetype.description();
        //     let tag_storage = archetype.tags();
        //     tag_slice_buffer.clear();
        //     for (tag_type, _) in desc.tags() {
        //         let storage = tag_storage
        //             .get(*tag_type)
        //             .expect("no tag storage for tag type");
        //         tag_slice_buffer.push((tag_type, storage));
        //     }
        //     for (idx, chunkset) in archetype.chunksets().iter().enumerate() {
        //         for chunk in chunkset.occupied() {
        //             let chunk_seq = serializer.serialize_seq(Some(desc.components().len()))?;
        //             for (comp_type, _) in desc.components() {
        //                 let comp_resources = chunk
        //                     .components(*comp_type)
        //                     .expect("no storage for component type in descriptor");
        //                 chunk_seq.serialize_element(&ComponentResourceSetSerializer {
        //                     world_serializer: self.world_serializer,
        //                     comp_resources,
        //                     comp_type,
        //                 })?;
        //             }
        //             chunk_seq.end();
        //         }
        //     }
        // }
    }
}

struct ArchetypeSerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    archetype: &'a ArchetypeData,
}
impl<'a, 'b, CS: WorldSerializer> Serialize for ArchetypeSerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut archetype = serializer.serialize_struct("Archetype", 3)?;
        let desc = self.archetype.description();
        archetype.serialize_field(
            "description",
            &ArchetypeDescriptionSerializer {
                world_serializer: self.world_serializer,
                desc,
            },
        )?;
        let tags: Vec<_> = desc
            .tags()
            .iter()
            .map(|(ty, meta)| {
                let tag_storage = self
                    .archetype
                    .tags()
                    .get(*ty)
                    .expect("tag type in archetype but not in storage");
                TagSerializer {
                    world_serializer: self.world_serializer,
                    ty,
                    meta,
                    tag_storage,
                }
            })
            .collect();
        archetype.serialize_field("tags", &tags)?;
        let chunksets: Vec<_> = self
            .archetype
            .chunksets()
            .iter()
            .map(|chunkset| {
                chunkset
                    .occupied()
                    .iter()
                    .map(|comp_storage| ChunkSerializer {
                        world_serializer: self.world_serializer,
                        desc,
                        comp_storage,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        archetype.serialize_field("chunk_sets", &chunksets)?;
        archetype.end()
    }
}

struct ArchetypeDescriptionSerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    desc: &'a ArchetypeDescription,
}
impl<'a, 'b, CS: WorldSerializer> Serialize for ArchetypeDescriptionSerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.world_serializer
            .serialize_archetype_description(serializer, self.desc)
    }
}

struct TagSerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    ty: &'a TagTypeId,
    meta: &'a TagMeta,
    tag_storage: &'a TagStorage,
}
impl<'a, 'b, CS: WorldSerializer> Serialize for TagSerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.world_serializer
            .serialize_tags(serializer, self.ty, self.meta, self.tag_storage)
    }
}

struct ChunkSerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    desc: &'a ArchetypeDescription,
    comp_storage: &'a ComponentStorage,
}
impl<'a, 'b, CS: WorldSerializer> Serialize for ChunkSerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut chunk = serializer.serialize_struct("Chunk", 2)?;
        chunk.serialize_field("entities", &EntitySerializer {
            world_serializer: self.world_serializer,
            entities: self.comp_storage.entities(),
        })?;
        let comp_storages: Vec<_> = self.desc
            .components()
            .iter()
            .map(|(ty, meta)| {
                let comp_resources = self
                    .comp_storage
                    .components(*ty)
                    .expect("component type in archetype but not in storage");
                ComponentResourceSetSerializer {
                    world_serializer: self.world_serializer,
                    ty,
                    meta,
                    comp_resources,
                }
            })
            .collect();
        chunk.serialize_field("components", &comp_storages)?;
        chunk.end()
    }
}

struct ComponentResourceSetSerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    ty: &'a ComponentTypeId,
    meta: &'a ComponentMeta,
    comp_resources: &'a ComponentResourceSet,
}
impl<'a, 'b, CS: WorldSerializer> Serialize for ComponentResourceSetSerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.world_serializer
            .serialize_components(serializer, self.ty, self.meta, self.comp_resources)
    }
}

struct EntitySerializer<'a, 'b, CS: WorldSerializer> {
    world_serializer: &'b CS,
    entities: &'a [Entity],
}
impl<'a, 'b, CS: WorldSerializer> Serialize for EntitySerializer<'a, 'b, CS> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.world_serializer
            .serialize_entities(serializer, self.entities)
    }
}
