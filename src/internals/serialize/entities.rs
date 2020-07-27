pub mod ser {
    use crate::internals::{
        query::filter::LayoutFilter,
        serialize::ser::WorldSerializer,
        storage::{
            archetype::{Archetype, ArchetypeIndex},
            component::ComponentTypeId,
            ComponentIndex,
        },
        world::World,
    };
    use itertools::Itertools;
    use serde::{ser::SerializeMap, Serialize, Serializer};
    use std::{collections::HashMap, marker::PhantomData};

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
    use crate::internals::{
        entity::Entity,
        serialize::de::WorldDeserializer,
        storage::{archetype::EntityLayout, component::ComponentTypeId, ComponentIndex},
        world::World,
    };
    use serde::{
        de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor},
        Deserializer,
    };
    use std::collections::HashMap;

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
                        if let Some(type_id) = self.world_deserializer.unmap_id(&mapped_type_id) {
                            self.world_deserializer
                                .register_component(mapped_type_id, &mut layout);
                            components.insert(
                                type_id,
                                map.next_value_seed(ComponentDeserializer {
                                    type_id,
                                    world_deserializer: self.world_deserializer,
                                })?,
                            );
                        } else {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }

                    let arch_index = self.world.get_or_insert_archetype(layout);
                    let comp_index = self.world.archetypes()[arch_index].entities().len();
                    self.world.archetypes_mut()[arch_index].push(self.entity);
                    self.world.entities_mut().insert(
                        &[self.entity],
                        arch_index,
                        ComponentIndex(comp_index),
                    );

                    for (type_id, component) in components {
                        let storage = self.world.components_mut().get_mut(type_id).unwrap();
                        unsafe { storage.extend_memcopy_raw(arch_index, component.as_ptr(), 1) };
                    }

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
