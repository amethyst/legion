use crate::{
    storage::{
        archetype::{ArchetypeIndex, EntityLayout},
        component::{Component, ComponentTypeId},
        UnknownComponentStorage,
    },
    world::World,
};
use de::WorldDeserializer;
use ser::WorldSerializer;
use serde::de::DeserializeSeed;
use std::{collections::HashMap, hash::Hash, marker::PhantomData};

pub mod de;
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
        "type_id": {
            "arch_index": [
                component,
                ...
            ]
        },
    ]
}
*/

#[doc(hidden)]
pub trait TypeKey:
    serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone + Hash
{
}

impl<T> TypeKey for T where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone + Hash
{
}

pub struct Serializer<T = SerializableTypeId>
where
    T: TypeKey,
{
    _phantom: PhantomData<T>,
    serialize_fns: HashMap<
        ComponentTypeId,
        (
            T,
            Box<dyn Fn(*const u8, usize, &mut dyn FnMut(&dyn erased_serde::Serialize))>,
            Box<
                dyn Fn(
                    &mut dyn UnknownComponentStorage,
                    ArchetypeIndex,
                    &mut dyn erased_serde::Deserializer,
                ) -> Result<(), erased_serde::Error>,
            >,
        ),
    >,
    constructors: HashMap<T, (ComponentTypeId, Box<dyn Fn(&mut EntityLayout)>)>,
}

impl<T> Serializer<T>
where
    T: TypeKey,
{
    pub fn new() -> Self {
        Self {
            serialize_fns: HashMap::new(),
            constructors: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    pub fn register<C: Component + serde::Serialize + for<'de> serde::Deserialize<'de>>(
        &mut self,
        mapped_type_id: T,
    ) {
        let type_id = ComponentTypeId::of::<C>();
        let serialize_fn = |ptr, count, serialize: &mut dyn FnMut(&dyn erased_serde::Serialize)| {
            let slice = unsafe { std::slice::from_raw_parts(ptr as *const C, count) };
            (serialize)(&slice);
        };
        let deserialize_fn =
            |storage: &mut dyn UnknownComponentStorage,
             arch: ArchetypeIndex,
             deserializer: &mut dyn erased_serde::Deserializer| {
                let mut components = erased_serde::deserialize::<Vec<C>>(deserializer)?;
                unsafe {
                    let ptr = components.as_ptr();
                    storage.extend_memcopy_raw(arch, ptr as *const u8, components.len());
                    components.set_len(0);
                }
                Ok(())
            };
        let constructor_fn = |layout: &mut EntityLayout| layout.register_component::<C>();
        self.serialize_fns.insert(
            type_id,
            (
                mapped_type_id.clone(),
                Box::new(serialize_fn),
                Box::new(deserialize_fn),
            ),
        );
        self.constructors
            .insert(mapped_type_id, (type_id, Box::new(constructor_fn)));
    }

    pub fn register_auto_mapped<
        C: Component + serde::Serialize + for<'de> serde::Deserialize<'de>,
    >(
        &mut self,
    ) where
        T: From<ComponentTypeId>,
    {
        self.register::<C>(ComponentTypeId::of::<C>().into())
    }
}

impl<T> WorldSerializer for Serializer<T>
where
    T: TypeKey,
{
    type TypeId = T;

    unsafe fn serialize_component_slice<S: serde::Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        count: usize,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Some((_, serialize_fn, _)) = self.serialize_fns.get(&ty) {
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
            .map(|(type_id, _, _)| type_id.clone())
    }
}

impl<T> WorldDeserializer for Serializer<T>
where
    T: TypeKey,
{
    type TypeId = T;

    fn unmap_id(&self, type_id: Self::TypeId) -> Option<ComponentTypeId> {
        self.constructors.get(&type_id).map(|(id, _)| *id)
    }

    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout) {
        if let Some((_, constructor)) = self.constructors.get(&type_id) {
            (constructor)(layout);
        }
    }

    fn deserialize_component_slice<'de, D: serde::Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        storage: &mut dyn UnknownComponentStorage,
        arch_index: ArchetypeIndex,
        deserializer: D,
    ) -> Result<(), D::Error> {
        if let Some((_, _, deserialize)) = self.serialize_fns.get(&type_id) {
            use serde::de::Error;
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            (deserialize)(storage, arch_index, &mut deserializer).map_err(D::Error::custom)
        } else {
            panic!();
        }
    }
}

impl<'de, T: TypeKey> DeserializeSeed<'de> for Serializer<T> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wrapped = de::Wrapper(self);
        wrapped.deserialize(deserializer)
    }
}

#[derive(
    Copy, Clone, PartialOrd, Ord, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
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
    use crate::{
        query::filter::filter_fns::any,
        world::{EntityStore, World},
    };
    use serde_json::json;

    #[test]
    fn serialize() {
        let mut world = World::default();

        let entity = world.extend(vec![
            (1usize, false, 1isize),
            (2usize, false, 2isize),
            (3usize, false, 3isize),
            (4usize, false, 4isize),
        ])[0];

        world.extend(vec![
            (5usize, 5isize),
            (6usize, 6isize),
            (7usize, 7isize),
            (8usize, 8isize),
        ]);

        let mut serializer = Serializer::<String>::new();
        serializer.register::<usize>("usize".to_string());
        serializer.register::<bool>("bool".to_string());
        serializer.register::<isize>("isize".to_string());

        let json = serde_json::to_value(&as_serializable(&world, any(), &serializer)).unwrap();
        println!("{:#}", json);

        let expected = json!({
          "archetypes": [
            {
              "components": [
                "usize",
                "bool",
                "isize"
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
                "isize"
              ],
              "entities": [
                127,
                126,
                125,
                124
              ]
            }
          ],
          "components": {
            "bool": {
              "0": [
                false,
                false,
                false,
                false
              ]
            },
            "isize": {
                "0": [
                  1,
                  2,
                  3,
                  4
                ],
                "1": [
                  5,
                  6,
                  7,
                  8
                ]
              },
            "usize": {
              "0": [
                1,
                2,
                3,
                4
              ],
              "1": [
                5,
                6,
                7,
                8
              ]
            }
          }
        });

        assert_eq!(json, expected);

        use serde::de::DeserializeSeed;
        let world: World = serializer.deserialize(json).unwrap();
        let entity = world.entry_ref(entity).unwrap();
        assert_eq!(entity.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entity.get_component::<bool>().unwrap(), &false);
        assert_eq!(entity.get_component::<isize>().unwrap(), &1isize);
    }
}
