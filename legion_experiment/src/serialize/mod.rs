use crate::storage::component::{Component, ComponentTypeId};
use ser::WorldSerializer;
use std::{collections::HashMap, marker::PhantomData};

pub mod ser;

/*
{
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

pub struct Serializer<T = SerializableTypeId>
where
    T: serde::Serialize + Ord + Clone,
{
    _phantom: PhantomData<T>,
    serialize_fns: HashMap<
        ComponentTypeId,
        (
            T,
            Box<dyn Fn(*const u8, usize, &mut dyn FnMut(&dyn erased_serde::Serialize))>,
        ),
    >,
}

impl<T> Serializer<T>
where
    T: serde::Serialize + Ord + Clone,
{
    pub fn new() -> Self {
        Self {
            serialize_fns: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    pub fn register<C: Component + serde::Serialize>(&mut self, mapped_type_id: T) {
        let type_id = ComponentTypeId::of::<C>();
        let serialize_fn = |ptr, count, serialize: &mut dyn FnMut(&dyn erased_serde::Serialize)| {
            let slice = unsafe { std::slice::from_raw_parts(ptr as *const C, count) };
            (serialize)(&slice);
        };
        self.serialize_fns
            .insert(type_id, (mapped_type_id, Box::new(serialize_fn)));
    }

    pub fn register_auto_mapped<C: Component + serde::Serialize>(&mut self)
    where
        T: From<ComponentTypeId>,
    {
        self.register::<C>(ComponentTypeId::of::<C>().into())
    }
}

impl<T> WorldSerializer for Serializer<T>
where
    T: serde::Serialize + Ord + Clone,
{
    type TypeId = T;

    unsafe fn serialize_component_slice<S: serde::Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        count: usize,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Some((_, serialize_fn)) = self.serialize_fns.get(&ty) {
            let mut serializer = Some(serializer);
            let mut result = None;
            let result_ref = &mut result;
            (serialize_fn)(ptr, count, &mut move |serializable| {
                *result_ref = Some(erased_serde::serialize(
                    serializable,
                    serializer
                        .take()
                        .expect("serialize can only be called once"),
                ));
            });
            result.unwrap()
        } else {
            panic!();
        }
    }

    fn map_id(&self, type_id: ComponentTypeId) -> Option<Self::TypeId> {
        self.serialize_fns
            .get(&type_id)
            .map(|(type_id, _)| type_id.clone())
    }
}

#[derive(Copy, Clone, PartialOrd, Ord, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SerializableTypeId(u64);

impl From<ComponentTypeId> for SerializableTypeId {
    fn from(type_id: ComponentTypeId) -> Self {
        let id = unsafe { std::mem::transmute(type_id.type_id) };
        Self(id)
    }
}

impl Into<ComponentTypeId> for SerializableTypeId {
    fn into(self) -> ComponentTypeId {
        let id = unsafe { std::mem::transmute(self.0) };
        ComponentTypeId::of_id(id)
    }
}

#[cfg(test)]
mod test {
    use super::{ser::as_serializable, Serializer};
    use crate::{query::filter::filter_fns::any, world::World};
    use serde_json::json;

    #[test]
    fn serialize() {
        let mut world = World::default();

        world.extend(vec![
            (1usize, false, "a"),
            (2usize, false, "b"),
            (3usize, false, "c"),
            (4usize, false, "d"),
        ]);

        world.extend(vec![
            (5usize, "e"),
            (6usize, "f"),
            (7usize, "g"),
            (8usize, "h"),
        ]);

        let mut serializer = Serializer::<String>::new();
        serializer.register::<usize>("usize".to_string());
        serializer.register::<bool>("bool".to_string());
        serializer.register::<&'static str>("&str".to_string());

        let json = serde_json::to_value(&as_serializable(&world, any(), serializer)).unwrap();
        println!("{:#}", json);

        let expected = json!({
          "archetypes": [
            {
              "components": [
                "usize",
                "bool",
                "&str"
              ],
              "entities": [
                63,
                62,
                61,
                60
              ]
            },
            {
              "components": [
                "usize",
                "&str"
              ],
              "entities": [
                127,
                126,
                125,
                124
              ]
            }
          ],
          "components": [
            {
              "archetypes": [
                {
                  "archetype": 0,
                  "slice": [
                    "a",
                    "b",
                    "c",
                    "d"
                  ]
                },
                {
                  "archetype": 1,
                  "slice": [
                    "e",
                    "f",
                    "g",
                    "h"
                  ]
                }
              ],
              "type": "&str"
            },
            {
              "archetypes": [
                {
                  "archetype": 0,
                  "slice": [
                    false,
                    false,
                    false,
                    false
                  ]
                }
              ],
              "type": "bool"
            },
            {
              "archetypes": [
                {
                  "archetype": 0,
                  "slice": [
                    1,
                    2,
                    3,
                    4
                  ]
                },
                {
                  "archetype": 1,
                  "slice": [
                    5,
                    6,
                    7,
                    8
                  ]
                }
              ],
              "type": "usize"
            }
          ]
        });

        assert_eq!(json, expected);
    }
}
