/*
{
    type_mappings: [
        (type_id, mapped),
    ],
    archetypes: [
        {
            components: [type_id],
            entities: [Entity],
        }
        ...
    ],
    components: [
        type_id,
        archetypes: [
            {
                index,
                slice: [
                    component,
                    ...
                ]
            },
            ...
        ]
    ]
}
*/

use crate::{
    query::filter::LayoutFilter,
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::ComponentTypeId,
    },
    world::World,
};
use itertools::Itertools;
use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};

pub trait WorldSerializer {
    fn can_serialize_component(&self, ty: &ComponentTypeId) -> bool;
    fn serialize_type_mapping<S: SerializeSeq>(
        &self,
        seq: S,
        ty: &ComponentTypeId,
    ) -> Result<S::Ok, S::Error>;
}

pub fn serialize_world<S, F, W>(
    serializer: S,
    world: &World,
    filter: F,
    world_serializer: W,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    F: LayoutFilter,
    W: WorldSerializer,
{
    let archetypes = world
        .archetypes()
        .iter()
        .enumerate()
        .filter(|(_, arch)| {
            filter
                .matches_layout(arch.layout().component_types())
                .is_pass()
        })
        .map(|(i, arch)| (ArchetypeIndex(i as u32), arch))
        .collect::<Vec<_>>();

    let component_types = archetypes
        .iter()
        .flat_map(|(_, arch)| arch.layout().component_types())
        .unique()
        .filter(|t| world_serializer.can_serialize_component(t))
        .copied()
        .collect::<Vec<_>>();

    let mut root = serializer.serialize_struct("World", 3)?;

    // serialize type mappings
    root.serialize_field(
        "type_mappings",
        &TypeMappings {
            component_types: component_types.as_slice(),
            world_serializer: &world_serializer,
        },
    )?;

    // serialize archetypes
    root.serialize_field("archetypes", todo!())?;

    // serialize components
    root.serialize_field("components", todo!())?;

    root.end()
}

struct TypeMappings<'a, W: WorldSerializer> {
    component_types: &'a [ComponentTypeId],
    world_serializer: &'a W,
}

impl<'a, W: WorldSerializer> Serialize for TypeMappings<'a, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.component_types.len()))?;
        for type_id in self.component_types.iter() {
            self.world_serializer.serialize_type_mapping(seq, type_id)?;
        }
        seq.end()
    }
}

struct SerializableArchetype<'a> {
    archetype: &'a Archetype,
}

impl<'a> Serialize for SerializableArchetype<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut root = serializer.serialize_struct("Archetype", 2)?;
        root.serialize_field("components", &self.archetype.layout().component_types())?;
        root.serialize_field("entities", &self.archetype.entities())?;
        root.end()
    }
}
