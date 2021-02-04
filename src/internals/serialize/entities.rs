pub mod ser {
    use std::{collections::HashMap, marker::PhantomData};

    use itertools::Itertools;
    use serde::{ser::SerializeMap, Serialize, Serializer};

    use crate::internals::{
        query::filter::LayoutFilter,
        serialize::{ser::WorldSerializer, UnknownType},
        storage::{
            archetype::{Archetype, ArchetypeIndex},
            component::ComponentTypeId,
            ComponentIndex,
        },
        world::World,
    };

    pub struct EntitiesLayoutSerializer<'a, W: WorldSerializer, F: LayoutFilter> {
        pub world_serializer: &'a W,
        pub world: &'a World,
        pub filter: &'a F,
    }

    impl<'a, W: WorldSerializer, F: LayoutFilter> Serialize for EntitiesLayoutSerializer<'a, W, F> {
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

            let component_types = archetypes
                .iter()
                .flat_map(|(_, arch)| arch.layout().component_types())
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

            let mut map = serializer.serialize_map(Some(
                archetypes
                    .iter()
                    .map(|(_, arch)| arch.entities().len())
                    .sum(),
            ))?;

            for (arch_index, arch) in archetypes {
                for (i, entity) in arch.entities().iter().enumerate() {
                    map.serialize_entry(
                        entity,
                        &EntitySerializer {
                            component_index: ComponentIndex(i),
                            arch_index,
                            arch,
                            world: self.world,
                            world_serializer: self.world_serializer,
                            type_mappings: &type_mappings,
                        },
                    )?;
                }
            }

            map.end()
        }
    }

    struct EntitySerializer<'a, W: WorldSerializer> {
        component_index: ComponentIndex,
        arch_index: ArchetypeIndex,
        arch: &'a Archetype,
        world: &'a World,
        world_serializer: &'a W,
        type_mappings: &'a HashMap<ComponentTypeId, W::TypeId>,
    }

    impl<'a, W: WorldSerializer> Serialize for EntitySerializer<'a, W> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let layout = self.arch.layout().component_types();
            let components = self.world.components();
            let type_count = layout
                .iter()
                .filter_map(|t| self.type_mappings.get(t))
                .count();

            let mut map = serializer.serialize_map(Some(type_count))?;

            for type_id in self.arch.layout().component_types() {
                if let Some(mapped_type_id) = self.type_mappings.get(type_id) {
                    let storage = components.get(*type_id).unwrap();
                    let (ptr, len) = storage.get_raw(self.arch_index).unwrap();
                    assert!(self.component_index.0 < len);
                    map.serialize_entry(
                        mapped_type_id,
                        &ComponentSerializer {
                            type_id: *type_id,
                            world_serializer: self.world_serializer,
                            ptr: unsafe {
                                ptr.add(self.component_index.0 * storage.element_vtable().size())
                            },
                            _phantom: PhantomData,
                        },
                    )?;
                }
            }

            map.end()
        }
    }

    struct ComponentSerializer<'a, W: WorldSerializer> {
        type_id: ComponentTypeId,
        ptr: *const u8,
        world_serializer: &'a W,
        _phantom: PhantomData<&'a u8>,
    }

    impl<'a, W: WorldSerializer> Serialize for ComponentSerializer<'a, W> {
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
    use std::{collections::HashMap, rc::Rc};

    use serde::{
        de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor},
        Deserializer,
    };

    use crate::internals::{
        entity::Entity,
        insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource},
        serialize::{de::WorldDeserializer, UnknownType},
        storage::{archetype::EntityLayout, component::ComponentTypeId},
        world::World,
    };

    pub struct EntitiesLayoutDeserializer<'a, W: WorldDeserializer> {
        pub world_deserializer: &'a W,
        pub world: &'a mut World,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for EntitiesLayoutDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct EntitySeqVisitor<'b, D: WorldDeserializer> {
                world_deserializer: &'b D,
                world: &'b mut World,
            }

            impl<'b, 'de, D: WorldDeserializer> Visitor<'de> for EntitySeqVisitor<'b, D> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("entity map")
                }

                fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    while let Some(entity) = map.next_key()? {
                        self.world.remove(entity);
                        map.next_value_seed(EntityDeserializer {
                            world_deserializer: self.world_deserializer,
                            world: self.world,
                            entity,
                        })?
                    }
                    Ok(())
                }
            }

            deserializer.deserialize_map(EntitySeqVisitor {
                world_deserializer: self.world_deserializer,
                world: self.world,
            })
        }
    }

    struct EntityDeserializer<'a, W: WorldDeserializer> {
        world_deserializer: &'a W,
        world: &'a mut World,
        entity: Entity,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for EntityDeserializer<'a, W> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct ComponentMapVisitor<'b, D: WorldDeserializer> {
                world_deserializer: &'b D,
                world: &'b mut World,
                entity: Entity,
            }

            impl<'b, 'de, D: WorldDeserializer> Visitor<'de> for ComponentMapVisitor<'b, D> {
                type Value = ();

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("struct Entity")
                }

                fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                where
                    V: MapAccess<'de>,
                {
                    let mut layout = EntityLayout::new();
                    let mut components = HashMap::new();

                    while let Some(mapped_type_id) = map.next_key::<D::TypeId>()? {
                        match self.world_deserializer.unmap_id(&mapped_type_id) {
                            Ok(type_id) => {
                                self.world_deserializer
                                    .register_component(mapped_type_id, &mut layout);
                                components.insert(
                                    type_id,
                                    map.next_value_seed(ComponentDeserializer {
                                        type_id,
                                        world_deserializer: self.world_deserializer,
                                    })?,
                                );
                            }
                            Err(missing) => {
                                match missing {
                                    UnknownType::Ignore => {
                                        map.next_value::<IgnoredAny>()?;
                                    }
                                    UnknownType::Error => {
                                        return Err(serde::de::Error::custom(
                                            "unknown component type",
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    struct SingleEntity {
                        entity: Entity,
                        components: HashMap<ComponentTypeId, Box<[u8]>>,
                        layout: Rc<EntityLayout>,
                    }

                    impl ArchetypeSource for SingleEntity {
                        type Filter = Rc<EntityLayout>;

                        fn filter(&self) -> Self::Filter {
                            self.layout.clone()
                        }

                        fn layout(&mut self) -> EntityLayout {
                            (*self.layout).clone()
                        }
                    }

                    impl ComponentSource for SingleEntity {
                        fn push_components<'a>(
                            &mut self,
                            writer: &mut ArchetypeWriter<'a>,
                            _: impl Iterator<Item = Entity>,
                        ) {
                            writer.push(self.entity);
                            for (type_id, component) in self.components.drain() {
                                let mut storage = writer.claim_components_unknown(type_id);
                                unsafe { storage.extend_memcopy_raw(component.as_ptr(), 1) };
                            }
                        }
                    }

                    impl IntoComponentSource for SingleEntity {
                        type Source = Self;
                        fn into(self) -> Self::Source {
                            self
                        }
                    }

                    self.world.extend(SingleEntity {
                        entity: self.entity,
                        components,
                        layout: Rc::new(layout),
                    });

                    Ok(())
                }
            }

            deserializer.deserialize_map(ComponentMapVisitor {
                world_deserializer: self.world_deserializer,
                world: self.world,
                entity: self.entity,
            })
        }
    }

    struct ComponentDeserializer<'a, W: WorldDeserializer> {
        type_id: ComponentTypeId,
        world_deserializer: &'a W,
    }

    impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for ComponentDeserializer<'a, W> {
        type Value = Box<[u8]>;

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            self.world_deserializer
                .deserialize_component(self.type_id, deserializer)
        }
    }
}
