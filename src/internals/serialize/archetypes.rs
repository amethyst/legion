pub mod ser {
    use std::collections::HashMap;

    use itertools::Itertools;
    use serde::{
        ser::{SerializeMap, SerializeSeq, SerializeStruct},
        Serialize, Serializer,
    };

    use crate::internals::{
        query::filter::LayoutFilter,
        serialize::{ser::WorldSerializer, UnknownType},
        storage::{
            archetype::{Archetype, ArchetypeIndex},
            component::ComponentTypeId,
            UnknownComponentStorage,
        },
        world::World,
    };

    pub struct ArchetypeLayoutSerializer<'a, W: WorldSerializer, F: LayoutFilter> {
        pub world_serializer: &'a W,
        pub world: &'a World,
        pub filter: &'a F,
    }

    impl<'a, W: WorldSerializer, F: LayoutFilter> Serialize for ArchetypeLayoutSerializer<'a, W, F> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let archetypes = self
                .world
                .archetypes()
                .iter()
                .filter(|arch| {
                    self.filter
                        .matches_layout(arch.layout().component_types())
                        .is_pass()
                })
                .collect_vec();

            let component_types = archetypes
                .iter()
                .flat_map(|arch| arch.layout().component_types())
                .unique();
            let mut type_mappings = HashMap::new();
            for id in component_types {
                match self.world_serializer.map_id(*id) {
                    Ok(type_id) => {
                        type_mappings.insert(*id, type_id);
                    }
                    Err(error) => {
                        match error {
                            UnknownType::Ignore => {}
                            UnknownType::Error => {
                                return Err(serde::ser::Error::custom(format!(
                                    "unknown component type {:?}",
                                    *id
                                )));
                            }
                        }
                    }
                }
            }

            let mut root = serializer.serialize_seq(Some(archetypes.len()))?;

            for archetype in archetypes {
                root.serialize_element(&SerializableArchetype {
                    world_serializer: self.world_serializer,
                    world: self.world,
                    type_mappings: &type_mappings,
                    archetype,
                })?;
            }

            root.end()
        }
    }

    struct SerializableArchetype<'a, W: WorldSerializer> {
        world_serializer: &'a W,
        world: &'a World,
        type_mappings: &'a HashMap<ComponentTypeId, W::TypeId>,
        archetype: &'a Archetype,
    }

    impl<'a, W: WorldSerializer> Serialize for SerializableArchetype<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut root = serializer.serialize_struct("archetype", 3)?;

            let mut component_type_ids = Vec::new();
            let mut component_external_ids = Vec::new();
            for type_id in self.archetype.layout().component_types() {
                if let Some(mapped) = self.type_mappings.get(type_id) {
                    component_type_ids.push(*type_id);
                    component_external_ids.push(mapped);
                }
            }

            root.serialize_field("_layout", &component_external_ids)?;
            root.serialize_field("entities", self.archetype.entities())?;
            root.serialize_field(
                "components",
                &SerializableArchetypeComponents {
                    world_serializer: self.world_serializer,
                    world: self.world,
                    component_external_ids,
                    component_type_ids,
                    archetype: self.archetype.index(),
                },
            )?;

            root.end()
        }
    }

    struct SerializableArchetypeComponents<'a, W: WorldSerializer> {
        world_serializer: &'a W,
        world: &'a World,
        component_type_ids: Vec<ComponentTypeId>,
        component_external_ids: Vec<&'a W::TypeId>,
        archetype: ArchetypeIndex,
    }

    impl<'a, W: WorldSerializer> Serialize for SerializableArchetypeComponents<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let components = self.world.components();

            let mut root = serializer.serialize_map(Some(self.component_external_ids.len()))?;

            for (i, external_id) in self.component_external_ids.iter().enumerate() {
                let type_id = self.component_type_ids[i];
                root.serialize_entry(
                    external_id,
                    &SerializableComponentSlice {
                        world_serializer: self.world_serializer,
                        storage: components.get(type_id).unwrap(),
                        archetype: self.archetype,
                        type_id,
                    },
                )?;
            }

            root.end()
        }
    }

    struct SerializableComponentSlice<'a, W: WorldSerializer> {
        world_serializer: &'a W,
        storage: &'a dyn UnknownComponentStorage,
        archetype: ArchetypeIndex,
        type_id: ComponentTypeId,
    }

    impl<'a, W: WorldSerializer> Serialize for SerializableComponentSlice<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            unsafe {
                self.world_serializer.serialize_component_slice(
                    self.type_id,
                    self.storage,
                    self.archetype,
                    serializer,
                )
            }
        }
    }
}

pub mod de {
    use std::{marker::PhantomData, rc::Rc};

    use serde::{
        de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
        Deserialize, Deserializer,
    };

    use crate::{
        internals::{
            serialize::de::WorldDeserializer,
            storage::{archetype::EntityLayout, component::ComponentTypeId},
            world::World,
        },
        storage::{ArchetypeSource, ComponentSource, IntoComponentSource, UnknownComponentWriter},
    };

    pub struct ArchetypeLayoutDeserializer<'a, W: WorldDeserializer> {
        pub world_deserializer: &'a W,
        pub world: &'a mut World,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for ArchetypeLayoutDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct SeqVisitor<'b, S: WorldDeserializer> {
                world_deserializer: &'b S,
                world: &'b mut World,
            }

            impl<'b, 'de, S: WorldDeserializer> Visitor<'de> for SeqVisitor<'b, S> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("archetype sequence")
                }

                fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
                where
                    V: SeqAccess<'de>,
                {
                    while seq
                        .next_element_seed(DeserializableArchetype {
                            world_deserializer: self.world_deserializer,
                            world: self.world,
                        })?
                        .is_some()
                    {}

                    Ok(())
                }
            }

            deserializer.deserialize_seq(SeqVisitor {
                world_deserializer: self.world_deserializer,
                world: self.world,
            })
        }
    }

    struct DeserializableArchetype<'a, W: WorldDeserializer> {
        world_deserializer: &'a W,
        world: &'a mut World,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for DeserializableArchetype<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct StructVisitor<'b, S: WorldDeserializer> {
                world_deserializer: &'b S,
                world: &'b mut World,
            }

            impl<'b, 'de, S: WorldDeserializer> Visitor<'de> for StructVisitor<'b, S> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("archetype sequence")
                }

                fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
                where
                    V: SeqAccess<'de>,
                {
                    use serde::de::Error;

                    let layout = seq
                        .next_element_seed(LayoutDeserializer {
                            world_deserializer: self.world_deserializer,
                        })?
                        .ok_or_else(|| V::Error::custom("expected archetype layout"))?;

                    let mut result = None;

                    self.world.extend(InsertArchetypeSeq {
                        world_deserializer: self.world_deserializer,
                        result: &mut result,
                        seq,
                        layout: Rc::new(layout),
                        _phantom: PhantomData,
                    });

                    match result.unwrap() {
                        Ok(_) => Ok(()),
                        Err(err) => Err(V::Error::custom(err)),
                    }
                }

                fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    use serde::de::Error;

                    let layout = if let Some(ArchetypeField::_Layout) = map.next_key()? {
                        map.next_value_seed(LayoutDeserializer {
                            world_deserializer: self.world_deserializer,
                        })
                    } else {
                        Err(V::Error::custom("expected archetype layout"))
                    }?;

                    let mut result = None;

                    self.world.extend(InsertArchetypeMap {
                        world_deserializer: self.world_deserializer,
                        result: &mut result,
                        map,
                        layout: Rc::new(layout),
                        _phantom: PhantomData,
                    });

                    match result.unwrap() {
                        Ok(_) => Ok(()),
                        Err(err) => Err(V::Error::custom(err)),
                    }
                }
            }

            const FIELDS: &[&str] = &["_layout", "entities", "components"];
            deserializer.deserialize_struct(
                "Archetype",
                FIELDS,
                StructVisitor {
                    world_deserializer: self.world_deserializer,
                    world: self.world,
                },
            )
        }
    }

    #[derive(Deserialize)]
    #[serde(field_identifier, rename_all = "lowercase")]
    enum ArchetypeField {
        _Layout,
        Entities,
        Components,
    }

    struct LayoutDeserializer<'a, W: WorldDeserializer> {
        world_deserializer: &'a W,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for LayoutDeserializer<'a, W> {
        type Value = EntityLayout;

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct SeqVisitor<'b, S: WorldDeserializer> {
                world_deserializer: &'b S,
            }

            impl<'b, 'de, S: WorldDeserializer> Visitor<'de> for SeqVisitor<'b, S> {
                type Value = EntityLayout;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("type ID sequence")
                }

                fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
                where
                    V: SeqAccess<'de>,
                {
                    let mut layout = EntityLayout::new();
                    while let Some(mapped_type_id) = seq.next_element::<S::TypeId>()? {
                        self.world_deserializer
                            .register_component(mapped_type_id, &mut layout);
                    }

                    Ok(layout)
                }
            }

            deserializer.deserialize_seq(SeqVisitor {
                world_deserializer: self.world_deserializer,
            })
        }
    }

    struct InsertArchetypeSeq<'a, 'de, W: WorldDeserializer, V: SeqAccess<'de>> {
        world_deserializer: &'a W,
        seq: V,
        result: &'a mut Option<Result<(), String>>,
        layout: Rc<EntityLayout>,
        _phantom: PhantomData<&'de ()>,
    }

    impl<'a, 'de, W: WorldDeserializer, V: SeqAccess<'de>> InsertArchetypeSeq<'a, 'de, W, V> {
        fn deserialize<'w>(
            &mut self,
            writer: &mut crate::storage::ArchetypeWriter<'w>,
        ) -> Result<(), V::Error> {
            use serde::de::Error;
            self.seq
                .next_element_seed(EntitySeq { writer })?
                .ok_or_else(|| V::Error::custom("expected entity seq"))?;
            self.seq
                .next_element_seed(ComponentMap {
                    writer,
                    world_deserializer: self.world_deserializer,
                })?
                .ok_or_else(|| V::Error::custom("expected component map"))?;
            Ok(())
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: SeqAccess<'de>> ArchetypeSource
        for InsertArchetypeSeq<'a, 'de, W, V>
    {
        type Filter = Rc<EntityLayout>;

        fn filter(&self) -> Self::Filter {
            self.layout.clone()
        }
        fn layout(&mut self) -> EntityLayout {
            (*self.layout).clone()
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: SeqAccess<'de>> ComponentSource
        for InsertArchetypeSeq<'a, 'de, W, V>
    {
        fn push_components<'c>(
            &mut self,
            writer: &mut crate::storage::ArchetypeWriter<'c>,
            _: impl Iterator<Item = crate::Entity>,
        ) {
            *self.result = match self.deserialize(writer) {
                Ok(_) => Some(Ok(())),
                Err(err) => Some(Err(err.to_string())),
            }
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: SeqAccess<'de>> IntoComponentSource
        for InsertArchetypeSeq<'a, 'de, W, V>
    {
        type Source = Self;
        fn into(self) -> Self::Source {
            self
        }
    }

    struct InsertArchetypeMap<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>> {
        world_deserializer: &'a W,
        map: V,
        result: &'a mut Option<Result<(), String>>,
        layout: Rc<EntityLayout>,
        _phantom: PhantomData<&'de ()>,
    }

    impl<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>> InsertArchetypeMap<'a, 'de, W, V> {
        fn deserialize<'w>(
            &mut self,
            writer: &mut crate::storage::ArchetypeWriter<'w>,
        ) -> Result<(), V::Error> {
            use serde::de::Error;
            while let Some(key) = self.map.next_key()? {
                match key {
                    ArchetypeField::Entities => self.map.next_value_seed(EntitySeq { writer })?,
                    ArchetypeField::Components => {
                        self.map.next_value_seed(ComponentMap {
                            writer,
                            world_deserializer: self.world_deserializer,
                        })?
                    }
                    _ => return Err(V::Error::custom("unexpected field")),
                }
            }

            Ok(())
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>> ArchetypeSource
        for InsertArchetypeMap<'a, 'de, W, V>
    {
        type Filter = Rc<EntityLayout>;

        fn filter(&self) -> Self::Filter {
            self.layout.clone()
        }
        fn layout(&mut self) -> EntityLayout {
            (*self.layout).clone()
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>> ComponentSource
        for InsertArchetypeMap<'a, 'de, W, V>
    {
        fn push_components<'c>(
            &mut self,
            writer: &mut crate::storage::ArchetypeWriter<'c>,
            _: impl Iterator<Item = crate::Entity>,
        ) {
            *self.result = match self.deserialize(writer) {
                Ok(_) => Some(Ok(())),
                Err(err) => Some(Err(err.to_string())),
            }
        }
    }

    impl<'a, 'de, W: WorldDeserializer, V: MapAccess<'de>> IntoComponentSource
        for InsertArchetypeMap<'a, 'de, W, V>
    {
        type Source = Self;
        fn into(self) -> Self::Source {
            self
        }
    }

    struct EntitySeq<'a, 'b> {
        writer: &'a mut crate::storage::ArchetypeWriter<'b>,
    }

    impl<'a, 'b, 'de> DeserializeSeed<'de> for EntitySeq<'a, 'b> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct SeqVisitor<'c, 'd> {
                writer: &'c mut crate::storage::ArchetypeWriter<'d>,
            }

            impl<'c, 'd, 'de> Visitor<'de> for SeqVisitor<'c, 'd> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("entity seq")
                }

                fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
                where
                    V: SeqAccess<'de>,
                {
                    if let Some(len) = seq.size_hint() {
                        self.writer.reserve(len);
                    }

                    while let Some(entity) = seq.next_element()? {
                        self.writer.push(entity);
                    }

                    Ok(())
                }
            }

            deserializer.deserialize_seq(SeqVisitor {
                writer: self.writer,
            })
        }
    }

    struct ComponentMap<'a, 'b, W: WorldDeserializer> {
        writer: &'a mut crate::storage::ArchetypeWriter<'b>,
        world_deserializer: &'a W,
    }

    impl<'a, 'b, 'de, W: WorldDeserializer> DeserializeSeed<'de> for ComponentMap<'a, 'b, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct MapVisitor<'c, 'd, S: WorldDeserializer> {
                writer: &'c mut crate::storage::ArchetypeWriter<'d>,
                world_deserializer: &'c S,
            }

            impl<'c, 'd, 'de, S: WorldDeserializer> Visitor<'de> for MapVisitor<'c, 'd, S> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("component map")
                }

                fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    while let Some(external_type_id) = map.next_key::<S::TypeId>()? {
                        if let Ok(type_id) = self.world_deserializer.unmap_id(&external_type_id) {
                            let storage = self.writer.claim_components_unknown(type_id);
                            map.next_value_seed(ComponentSlice {
                                storage,
                                type_id,
                                world_deserializer: self.world_deserializer,
                            })?;
                        }
                    }

                    Ok(())
                }
            }

            deserializer.deserialize_map(MapVisitor {
                world_deserializer: self.world_deserializer,
                writer: self.writer,
            })
        }
    }

    struct ComponentSlice<'a, 'b, W: WorldDeserializer> {
        storage: UnknownComponentWriter<'b>,
        type_id: ComponentTypeId,
        world_deserializer: &'a W,
    }

    impl<'a, 'b, 'de, W: WorldDeserializer> DeserializeSeed<'de> for ComponentSlice<'a, 'b, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            self.world_deserializer.deserialize_component_slice(
                self.type_id,
                self.storage,
                deserializer,
            )
        }
    }
}
