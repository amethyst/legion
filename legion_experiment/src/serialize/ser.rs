use crate::{
    query::filter::LayoutFilter,
    storage::{
        archetype::{Archetype, ArchetypeIndex},
        component::ComponentTypeId,
        UnknownComponentStorage,
    },
    world::World,
};
use itertools::Itertools;
use serde::ser::{Serialize, SerializeStruct, Serializer};
use std::{collections::HashMap, marker::PhantomData};

pub trait WorldSerializer {
    type TypeId: Serialize + Ord;

    fn map_id(&self, type_id: ComponentTypeId) -> Option<Self::TypeId>;
    unsafe fn serialize_component_slice<S: Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        count: usize,
        serializer: S,
    ) -> Result<S::Ok, S::Error>;
}

pub struct SerializableWorld<'a, F: LayoutFilter, W: WorldSerializer> {
    world: &'a World,
    filter: F,
    world_serializer: W,
}

impl<'a, F: LayoutFilter, W: WorldSerializer> Serialize for SerializableWorld<'a, F, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_world(
            serializer,
            &self.world,
            &self.filter,
            &self.world_serializer,
        )
    }
}

pub fn as_serializable<'a, F: LayoutFilter, W: WorldSerializer>(
    world: &'a World,
    filter: F,
    world_serializer: W,
) -> SerializableWorld<'a, F, W> {
    SerializableWorld {
        world,
        filter,
        world_serializer,
    }
}

pub fn serialize_world<S, F, W>(
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

    let type_mappings = archetypes
        .iter()
        .flat_map(|(_, arch)| arch.layout().component_types())
        .unique()
        .filter_map(|id| world_serializer.map_id(*id).map(|mapped| (*id, mapped)))
        .collect::<HashMap<ComponentTypeId, W::TypeId>>();

    let mut root = serializer.serialize_struct("World", 3)?;

    // serialize archetypes
    root.serialize_field(
        "archetypes",
        &archetypes
            .iter()
            .map(|(_, archetype)| SerializableArchetype {
                archetype,
                type_mappings: &type_mappings,
            })
            .collect::<Vec<_>>(),
    )?;

    // serialize components
    let mut components = type_mappings
        .into_iter()
        .map(|(type_id, mapped)| SerializableComponentStorage {
            storage: world.components().get(type_id).unwrap(),
            world_serializer: world_serializer,
            type_id,
            mapped,
            archetypes: &archetypes,
        })
        .collect::<Vec<_>>();
    components.sort_by(|a, b| a.mapped.cmp(&b.mapped));
    root.serialize_field("components", &components)?;

    root.end()
}

struct SerializableArchetype<'a, T: Serialize> {
    archetype: &'a Archetype,
    type_mappings: &'a HashMap<ComponentTypeId, T>,
}

impl<'a, T: Serialize> Serialize for SerializableArchetype<'a, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let component_types = self
            .archetype
            .layout()
            .component_types()
            .iter()
            .filter_map(|type_id| self.type_mappings.get(type_id))
            .collect::<Vec<&T>>();

        let mut root = serializer.serialize_struct("Archetype", 2)?;
        root.serialize_field("components", &component_types)?;
        root.serialize_field("entities", &self.archetype.entities())?;
        root.end()
    }
}

struct SerializableComponentStorage<'a, W: WorldSerializer> {
    storage: &'a dyn UnknownComponentStorage,
    world_serializer: &'a W,
    type_id: ComponentTypeId,
    mapped: W::TypeId,
    archetypes: &'a [(ArchetypeIndex, &'a Archetype)],
}

impl<'a, W: WorldSerializer> Serialize for SerializableComponentStorage<'a, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let slices = self
            .archetypes
            .iter()
            .enumerate()
            .filter_map(|(local_index, (arch_index, _))| {
                self.storage
                    .get_raw(*arch_index)
                    .map(|(ptr, len)| (local_index, ptr, len))
            })
            .map(|(arch_index, ptr, len)| SerializableArchetypeSlice {
                slice: SerializableSlice {
                    ptr,
                    len,
                    type_id: self.type_id,
                    world_serializer: self.world_serializer,
                    _phantom: PhantomData,
                },
                arch_index,
            })
            .collect::<Vec<_>>();

        let mut root = serializer.serialize_struct("Storage", 2)?;
        root.serialize_field("type", &self.mapped)?;
        root.serialize_field("archetypes", &slices)?;
        root.end()
    }
}

struct SerializableArchetypeSlice<'a, W: WorldSerializer> {
    arch_index: usize,
    slice: SerializableSlice<'a, W>,
}

impl<'a, W: WorldSerializer> Serialize for SerializableArchetypeSlice<'a, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut root = serializer.serialize_struct("ArchetypeSlice", 2)?;
        root.serialize_field("archetype", &self.arch_index)?;
        root.serialize_field("slice", &self.slice)?;
        root.end()
    }
}

struct SerializableSlice<'a, W: WorldSerializer> {
    type_id: ComponentTypeId,
    ptr: *const u8,
    len: usize,
    world_serializer: &'a W,
    _phantom: PhantomData<&'a u8>,
}

impl<'a, W: WorldSerializer> Serialize for SerializableSlice<'a, W> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        unsafe {
            self.world_serializer.serialize_component_slice(
                self.type_id,
                self.ptr,
                self.len,
                serializer,
            )
        }
    }
}
