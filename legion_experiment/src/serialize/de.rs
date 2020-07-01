use crate::{
    entity::Entity,
    storage::{
        archetype::{ArchetypeIndex, EntityLayout},
        component::ComponentTypeId,
        ComponentIndex, UnknownComponentStorage,
    },
    world::World,
};
use serde::{
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};

pub trait WorldDeserializer {
    type TypeId: for<'de> Deserialize<'de>;

    fn unmap_id(&self, type_id: Self::TypeId) -> Option<ComponentTypeId>;
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
        const FIELDS: &[&str] = &["archetypes", "components"];
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

impl<'de, W: WorldDeserializer> Visitor<'de> for WorldVisitor<W> {
    type Value = World;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("struct World")
    }

    fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let mut world = World::default();

        seq.next_element_seed(ArchetypeDeserializer {
            world: &mut world,
            world_deserializer: &self.world_deserializer,
        })?;

        seq.next_element_seed(ComponentsDeserializer {
            world: &mut world,
            world_deserializer: &self.world_deserializer,
        })?;

        Ok(world)
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut world = World::default();

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Archetypes,
            Components,
        }

        while let Some(key) = map.next_key()? {
            match key {
                Field::Archetypes => {
                    map.next_value_seed(ArchetypeListDeserializer {
                        world: &mut world,
                        world_deserializer: &self.world_deserializer,
                    })?;
                }
                Field::Components => {
                    map.next_value_seed(ComponentsDeserializer {
                        world: &mut world,
                        world_deserializer: &self.world_deserializer,
                    })?;
                }
            }
        }

        Ok(world)
    }
}

struct ArchetypeListDeserializer<'a, W: WorldDeserializer> {
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ArchetypeListDeserializer<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, 'a, W: WorldDeserializer> Visitor<'de> for ArchetypeListDeserializer<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("seqence of archetypes")
    }

    fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        while seq
            .next_element_seed(ArchetypeDeserializer {
                world: self.world,
                world_deserializer: self.world_deserializer,
            })?
            .is_some()
        {}

        Ok(())
    }
}

struct ArchetypeDeserializer<'a, W: WorldDeserializer> {
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ArchetypeDeserializer<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["entities", "components"];
        deserializer.deserialize_struct(
            "Archetype",
            FIELDS,
            ArchetypeVisitor {
                world: self.world,
                world_deserializer: self.world_deserializer,
            },
        )?;
        Ok(())
    }
}

struct ArchetypeVisitor<'a, W: WorldDeserializer> {
    world: &'a mut World,
    world_deserializer: &'a W,
}

impl<'a, W: WorldDeserializer> ArchetypeVisitor<'a, W> {
    fn create_archetype(&mut self, components: Vec<W::TypeId>, entities: Vec<Entity>) {
        let mut layout = EntityLayout::default();
        for component in components {
            self.world_deserializer
                .register_component(component, &mut layout);
        }

        let index = self.world.insert_archetype(layout);
        let base = self.world.archetypes()[index].entities().len();
        self.world
            .entities_mut()
            .insert(&entities, index, ComponentIndex(base));

        self.world.archetypes_mut()[index]
            .entities_mut()
            .extend(entities);
    }
}

impl<'de, 'a, W: WorldDeserializer> Visitor<'de> for ArchetypeVisitor<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("struct Archetype")
    }

    fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let components: Vec<W::TypeId> = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        let entities: Vec<Entity> = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;

        self.create_archetype(components, entities);

        Ok(())
    }

    fn visit_map<V>(mut self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: MapAccess<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Components,
            Entities,
        }

        let mut components: Option<Vec<W::TypeId>> = None;
        let mut entities: Option<Vec<Entity>> = None;
        while let Some(key) = map.next_key()? {
            match key {
                Field::Components => {
                    if components.is_some() {
                        return Err(serde::de::Error::duplicate_field("components"));
                    }
                    components = Some(map.next_value()?);
                }
                Field::Entities => {
                    if entities.is_some() {
                        return Err(serde::de::Error::duplicate_field("entities"));
                    }
                    entities = Some(map.next_value()?);
                }
            }
        }

        let components = components.ok_or_else(|| serde::de::Error::missing_field("components"))?;
        let entities = entities.ok_or_else(|| serde::de::Error::missing_field("entities"))?;
        self.create_archetype(components, entities);

        Ok(())
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
            if let Some(type_id) = self.world_deserializer.unmap_id(mapped_id) {
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
                arch_index: ArchetypeIndex(arch_index),
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
