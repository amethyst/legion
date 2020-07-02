use super::{ArchetypeDef, WorldMeta};
use crate::{
    storage::{
        archetype::{ArchetypeIndex, EntityLayout},
        component::ComponentTypeId,
        group::Group,
        ComponentIndex, UnknownComponentStorage,
    },
    world::{Universe, World, WorldOptions},
};
use serde::{
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};

pub trait WorldDeserializer {
    type TypeId: for<'de> Deserialize<'de>;

    fn unmap_id(&self, type_id: &Self::TypeId) -> Option<ComponentTypeId>;
    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout);
    fn deserialize_component_slice<'de, D: Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        storage: &mut dyn UnknownComponentStorage,
        arch_index: ArchetypeIndex,
        deserializer: D,
    ) -> Result<(), D::Error>;
}

pub(crate) struct Wrapper<T: WorldDeserializer>(pub T);

impl<'de, W: WorldDeserializer> DeserializeSeed<'de> for Wrapper<W> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["_meta", "archetypes", "components"];
        deserializer.deserialize_struct(
            "World",
            FIELDS,
            WorldVisitor {
                world_deserializer: self.0,
            },
        )
    }
}

struct WorldVisitor<W: WorldDeserializer> {
    world_deserializer: W,
}

impl<W: WorldDeserializer> WorldVisitor<W> {
    fn create_archetype(&mut self, world: &mut World, archetype: ArchetypeDef<W::TypeId>) {
        let mut layout = EntityLayout::default();
        for component in archetype.components {
            self.world_deserializer
                .register_component(component, &mut layout);
        }

        let index = world.insert_archetype(layout);
        let base = world.archetypes()[index].entities().len();
        world
            .entities_mut()
            .insert(&archetype.entities, index, ComponentIndex(base));

        world.archetypes_mut()[index]
            .entities_mut()
            .extend(archetype.entities);
    }
}

impl<'de, W: WorldDeserializer> Visitor<'de> for WorldVisitor<W> {
    type Value = World;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("struct World")
    }

    fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let mut world = seq
            .next_element_seed(MetaDeserializer {
                world_deserializer: &self.world_deserializer,
            })?
            .unwrap();

        let archetypes = seq.next_element::<Vec<ArchetypeDef<W::TypeId>>>()?.unwrap();
        for archetype in archetypes {
            self.create_archetype(&mut world, archetype);
        }

        seq.next_element_seed(ComponentsDeserializer {
            world: &mut world,
            world_deserializer: &self.world_deserializer,
        })?;

        Ok(world)
    }

    fn visit_map<V>(mut self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut world = None;

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            _Meta,
            Archetypes,
            Components,
        }

        while let Some(key) = map.next_key()? {
            match key {
                Field::_Meta => {
                    if world.is_some() {
                        return Err(serde::de::Error::duplicate_field("_meta"));
                    }
                    world = Some(map.next_value_seed(MetaDeserializer {
                        world_deserializer: &self.world_deserializer,
                    })?);
                }
                Field::Archetypes => {
                    if let Some(world) = &mut world {
                        let archetypes = map.next_value::<Vec<ArchetypeDef<W::TypeId>>>()?;
                        for archetype in archetypes {
                            self.create_archetype(world, archetype);
                        }
                    } else {
                        return Err(serde::de::Error::missing_field("_meta"));
                    }
                }
                Field::Components => {
                    if let Some(world) = &mut world {
                        map.next_value_seed(ComponentsDeserializer {
                            world,
                            world_deserializer: &self.world_deserializer,
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
                    Group::new(
                        group
                            .iter()
                            .filter_map(|id| self.world_deserializer.unmap_id(id)),
                    )
                })
                .collect(),
        };

        Ok(universe.create_world_with_options(options))
    }
}

struct ComponentsDeserializer<'a, W: WorldDeserializer> {
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ComponentsDeserializer<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ComponentsVisitor {
            world: self.world,
            world_deserializer: self.world_deserializer,
        })?;

        Ok(())
    }
}

struct ComponentsVisitor<'a, W: WorldDeserializer> {
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> Visitor<'de> for ComponentsVisitor<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("component map")
    }

    fn visit_map<V>(mut self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        while let Some(mapped_id) = map.next_key()? {
            if let Some(type_id) = self.world_deserializer.unmap_id(&mapped_id) {
                map.next_value_seed(ArchetypeSliceDeserializer {
                    type_id,
                    world: &mut self.world,
                    world_deserializer: self.world_deserializer,
                })?;
            }
        }

        Ok(())
    }
}

struct ArchetypeSliceDeserializer<'a, W: WorldDeserializer> {
    type_id: ComponentTypeId,
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ArchetypeSliceDeserializer<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(self)?;
        Ok(())
    }
}

impl<'de, 'a, W: WorldDeserializer> Visitor<'de> for ArchetypeSliceDeserializer<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("component slice map")
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        while let Some(arch_index) = map.next_key()? {
            let storage = self
                .world
                .components_mut()
                .get_mut(self.type_id)
                .expect("component storage missing");

            map.next_value_seed(SliceDeserializer {
                storage,
                arch_index,
                world_deserializer: self.world_deserializer,
                type_id: self.type_id,
            })?;
        }

        Ok(())
    }
}

struct SliceDeserializer<'a, W: WorldDeserializer> {
    storage: &'a mut dyn UnknownComponentStorage,
    world_deserializer: &'a W,
    arch_index: ArchetypeIndex,
    type_id: ComponentTypeId,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for SliceDeserializer<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.world_deserializer.deserialize_component_slice(
            self.type_id,
            self.storage,
            self.arch_index,
            deserializer,
        )?;
        Ok(())
    }
}
