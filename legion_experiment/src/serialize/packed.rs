use crate::entity::Entity;

#[derive(serde::Serialize, serde::Deserialize)]
struct ArchetypeDef<T> {
    pub components: Vec<T>,
    pub entities: Vec<Entity>,
}

pub mod ser {
    use super::ArchetypeDef;
    use crate::{
        query::filter::LayoutFilter,
        serialize::ser::WorldSerializer,
        storage::{
            archetype::{Archetype, ArchetypeIndex},
            component::ComponentTypeId,
            UnknownComponentStorage,
        },
        world::World,
    };
    use itertools::Itertools;
    use serde::{
        ser::{SerializeMap, SerializeSeq, SerializeStruct},
        Serialize, Serializer,
    };
    use std::{collections::HashMap, marker::PhantomData};

    pub struct PackedLayoutSerializer<'a, W: WorldSerializer, F: LayoutFilter> {
        pub world_serializer: &'a W,
        pub world: &'a World,
        pub filter: &'a F,
    }

    impl<'a, W: WorldSerializer, F: LayoutFilter> Serialize for PackedLayoutSerializer<'a, W, F> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let archetypes = self
                .world
                .archetypes()
                .iter()
                .enumerate()
                .filter(|(_, arch)| {
                    self.filter
                        .matches_layout(arch.layout().component_types())
                        .is_pass()
                })
                .map(|(i, arch)| (ArchetypeIndex(i as u32), arch))
                .collect::<Vec<_>>();

            let type_mappings = archetypes
                .iter()
                .flat_map(|(_, arch)| arch.layout().component_types())
                .unique()
                .filter_map(|id| {
                    self.world_serializer
                        .map_id(*id)
                        .map(|mapped| (*id, mapped))
                })
                .collect::<HashMap<ComponentTypeId, W::TypeId>>();

            let archetype_defs = archetypes
                .iter()
                .map(|(_, archetype)| {
                    let components = archetype
                        .layout()
                        .component_types()
                        .iter()
                        .filter_map(|type_id| type_mappings.get(type_id))
                        .collect::<Vec<_>>();
                    ArchetypeDef {
                        components,
                        entities: archetype.entities().to_vec(),
                    }
                })
                .collect::<Vec<_>>();

            let mut root = serializer.serialize_struct("Data", 2)?;

            // serialize archetypes
            root.serialize_field("archetypes", &archetype_defs)?;

            // serialize components
            root.serialize_field(
                "components",
                &Components {
                    world_serializer: self.world_serializer,
                    world: self.world,
                    type_mappings,
                    archetypes,
                },
            )?;

            root.end()
        }
    }

    struct Components<'a, W: WorldSerializer> {
        world_serializer: &'a W,
        world: &'a World,
        type_mappings: HashMap<ComponentTypeId, W::TypeId>,
        archetypes: Vec<(ArchetypeIndex, &'a Archetype)>,
    }

    impl<'a, W: WorldSerializer> Serialize for Components<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut components = self
                .type_mappings
                .iter()
                .map(|(type_id, mapped)| {
                    (
                        mapped,
                        SerializableComponentStorage {
                            storage: self.world.components().get(*type_id).unwrap(),
                            world_serializer: self.world_serializer,
                            type_id: *type_id,
                            archetypes: &self.archetypes,
                        },
                    )
                })
                .collect::<Vec<_>>();
            components.sort_by(|a, b| a.0.cmp(&b.0));

            let mut root = serializer.serialize_map(Some(components.len()))?;
            for (mapped, storage) in components {
                root.serialize_entry(mapped, &storage)?;
            }
            root.end()
        }
    }

    struct SerializableComponentStorage<'a, W: WorldSerializer> {
        storage: &'a dyn UnknownComponentStorage,
        world_serializer: &'a W,
        type_id: ComponentTypeId,
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
                    self.storage.get_raw(*arch_index).map(|(ptr, len)| {
                        (local_index, ptr, self.storage.element_vtable().size(), len)
                    })
                })
                .map(|(local_index, ptr, size, len)| {
                    (
                        ArchetypeIndex(local_index as u32),
                        SerializableSlice {
                            ptr,
                            size,
                            len,
                            type_id: self.type_id,
                            world_serializer: self.world_serializer,
                            _phantom: PhantomData,
                        },
                    )
                })
                .collect::<Vec<_>>();

            let mut root = serializer.serialize_map(Some(slices.len()))?;
            for (idx, slice) in slices {
                root.serialize_entry(&idx, &slice)?;
            }
            root.end()
        }
    }

    struct SerializableSlice<'a, W: WorldSerializer> {
        type_id: ComponentTypeId,
        ptr: *const u8,
        size: usize,
        len: usize,
        world_serializer: &'a W,
        _phantom: PhantomData<&'a u8>,
    }

    impl<'a, W: WorldSerializer> Serialize for SerializableSlice<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut seq = serializer.serialize_seq(Some(self.len))?;
            for i in 0..self.len {
                seq.serialize_element(&SerializeComponent {
                    type_id: self.type_id,
                    ptr: unsafe { self.ptr.add(i * self.size) },
                    world_serializer: self.world_serializer,
                    _phantom: PhantomData,
                })?;
            }

            seq.end()
        }
    }

    struct SerializeComponent<'a, W: WorldSerializer> {
        type_id: ComponentTypeId,
        ptr: *const u8,
        world_serializer: &'a W,
        _phantom: PhantomData<&'a u8>,
    }

    impl<'a, W: WorldSerializer> Serialize for SerializeComponent<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            unsafe {
                self.world_serializer
                    .serialize_component(self.type_id, self.ptr, serializer)
            }
        }
    }
}

pub mod de {
    use super::ArchetypeDef;
    use crate::{
        serialize::de::WorldDeserializer,
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

    pub struct PackedLayoutDeserializer<'a, W: WorldDeserializer> {
        pub world_deserializer: &'a W,
        pub world: &'a mut World,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for PackedLayoutDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct PackedVisitor<'b, S: WorldDeserializer> {
                world_deserializer: &'b S,
                world: &'b mut World,
            }

            impl<'b, S: WorldDeserializer> PackedVisitor<'b, S> {
                fn create_archetype(
                    &mut self,
                    archetype: ArchetypeDef<S::TypeId>,
                ) -> ArchetypeIndex {
                    let mut layout = EntityLayout::default();
                    for component in archetype.components {
                        self.world_deserializer
                            .register_component(component, &mut layout);
                    }

                    let index = self.world.insert_archetype(layout);
                    let base = self.world.archetypes()[index].entities().len();
                    self.world.entities_mut().insert(
                        &archetype.entities,
                        index,
                        ComponentIndex(base),
                    );

                    self.world.archetypes_mut()[index]
                        .entities_mut()
                        .extend(archetype.entities);

                    index
                }
            }

            impl<'b, 'de, S: WorldDeserializer> Visitor<'de> for PackedVisitor<'b, S> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("struct Data")
                }

                fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
                where
                    V: SeqAccess<'de>,
                {
                    let archetype_indexes = seq
                        .next_element::<Vec<ArchetypeDef<S::TypeId>>>()?
                        .unwrap()
                        .into_iter()
                        .map(|def| self.create_archetype(def))
                        .collect();

                    seq.next_element_seed(ComponentsDeserializer {
                        world: &mut self.world,
                        world_deserializer: self.world_deserializer,
                        archetype_indexes,
                    })?;

                    Ok(())
                }

                fn visit_map<V>(mut self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    #[derive(Deserialize)]
                    #[serde(field_identifier, rename_all = "lowercase")]
                    enum Field {
                        Archetypes,
                        Components,
                    }

                    let mut archetype_indexes = None;
                    while let Some(key) = map.next_key()? {
                        match key {
                            Field::Archetypes => {
                                if archetype_indexes.is_some() {
                                    return Err(serde::de::Error::duplicate_field("archetypes"));
                                }
                                archetype_indexes = Some(
                                    map.next_value::<Vec<ArchetypeDef<S::TypeId>>>()?
                                        .into_iter()
                                        .map(|def| self.create_archetype(def))
                                        .collect(),
                                )
                            }
                            Field::Components => {
                                map.next_value_seed(ComponentsDeserializer {
                                    world: &mut self.world,
                                    world_deserializer: self.world_deserializer,
                                    archetype_indexes: archetype_indexes.take().ok_or_else(
                                        || serde::de::Error::missing_field("archetypes"),
                                    )?,
                                })?;
                            }
                        }
                    }
                    Ok(())
                }
            }

            const FIELDS: &[&str] = &["archetypes", "components"];
            deserializer.deserialize_struct(
                "Data",
                FIELDS,
                PackedVisitor {
                    world_deserializer: self.world_deserializer,
                    world: self.world,
                },
            )
        }
    }

    struct ComponentsDeserializer<'a, W: WorldDeserializer> {
        world: &'a mut World,
        world_deserializer: &'a W,
        archetype_indexes: Vec<ArchetypeIndex>,
    }

    impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ComponentsDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct ComponentsVisitor<'b, D: WorldDeserializer> {
                world: &'b mut World,
                world_deserializer: &'b D,
                archetype_indexes: Vec<ArchetypeIndex>,
            }

            impl<'de, 'b, D: WorldDeserializer> Visitor<'de> for ComponentsVisitor<'b, D> {
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
                                archetype_indexes: &self.archetype_indexes,
                            })?;
                        }
                    }

                    Ok(())
                }
            }

            deserializer.deserialize_map(ComponentsVisitor {
                world: self.world,
                world_deserializer: self.world_deserializer,
                archetype_indexes: self.archetype_indexes,
            })?;

            Ok(())
        }
    }

    struct ArchetypeSliceDeserializer<'a, W: WorldDeserializer> {
        type_id: ComponentTypeId,
        world: &'a mut World,
        world_deserializer: &'a W,
        archetype_indexes: &'a [ArchetypeIndex],
    }

    impl<'de, 'a, W: WorldDeserializer> DeserializeSeed<'de> for ArchetypeSliceDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            impl<'de, 'a, W: WorldDeserializer> Visitor<'de> for ArchetypeSliceDeserializer<'a, W> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("component slice map")
                }

                fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    while let Some(local_arch_index) = map.next_key::<ArchetypeIndex>()? {
                        let storage = self
                            .world
                            .components_mut()
                            .get_mut(self.type_id)
                            .expect("component storage missing");
                        let arch_index = self.archetype_indexes[local_arch_index.0 as usize];

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

            deserializer.deserialize_map(self)?;
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
}
