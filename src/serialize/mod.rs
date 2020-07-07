//! Contains types required to serialize and deserialize a world via the serde library.

use crate::{
    storage::{ArchetypeIndex, Component, ComponentTypeId, EntityLayout, UnknownComponentStorage},
    world::World,
};
use serde::{de::DeserializeSeed, Serializer};
use std::{collections::HashMap, hash::Hash, marker::PhantomData};

mod de;
mod entities;
mod packed;
mod ser;

pub use de::WorldDeserializer;
pub use ser::{SerializableWorld, WorldSerializer};

/// A (de)serializable type which can represent a component type in a serialized world.
///
/// This trait has a blanket impl for all applicable types.
pub trait TypeKey:
    serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone + Hash
{
}

impl<T> TypeKey for T where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone + Hash
{
}

type SerializeFn = fn(*const u8, &mut dyn FnMut(&dyn erased_serde::Serialize));
type DeserializeSliceFn = fn(
    &mut dyn UnknownComponentStorage,
    ArchetypeIndex,
    &mut dyn erased_serde::Deserializer,
) -> Result<(), erased_serde::Error>;
type DeserializeSingleFn =
    fn(&mut dyn erased_serde::Deserializer) -> Result<Box<[u8]>, erased_serde::Error>;

/// A world (de)serializer which describes how to (de)serialize the component types in a world.
pub struct Registry<T = SerializableTypeId>
where
    T: TypeKey,
{
    _phantom: PhantomData<T>,
    serialize_fns:
        HashMap<ComponentTypeId, (T, SerializeFn, DeserializeSliceFn, DeserializeSingleFn)>,
    constructors: HashMap<T, (ComponentTypeId, fn(&mut EntityLayout))>,
}

impl<T> Registry<T>
where
    T: TypeKey,
{
    /// Constructs a new registry.
    pub fn new() -> Self {
        Self {
            serialize_fns: HashMap::new(),
            constructors: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Registers a component type and its key with the registry.
    pub fn register<C: Component + serde::Serialize + for<'de> serde::Deserialize<'de>>(
        &mut self,
        mapped_type_id: T,
    ) {
        let type_id = ComponentTypeId::of::<C>();
        let serialize_fn = |ptr, serialize: &mut dyn FnMut(&dyn erased_serde::Serialize)| {
            let component = unsafe { &*(ptr as *const C) };
            (serialize)(component);
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
        let deserialize_single_fn = |deserializer: &mut dyn erased_serde::Deserializer| {
            let component = erased_serde::deserialize::<C>(deserializer)?;
            unsafe {
                let vec = std::slice::from_raw_parts(
                    &component as *const C as *const u8,
                    std::mem::size_of::<C>(),
                )
                .to_vec();
                std::mem::forget(component);
                Ok(vec.into_boxed_slice())
            }
        };
        let constructor_fn = |layout: &mut EntityLayout| layout.register_component::<C>();
        self.serialize_fns.insert(
            type_id,
            (
                mapped_type_id.clone(),
                serialize_fn,
                deserialize_fn,
                deserialize_single_fn,
            ),
        );
        self.constructors
            .insert(mapped_type_id, (type_id, constructor_fn));
    }

    /// Registers a component type and its key with the registry.
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

impl<T: TypeKey> Default for Registry<T> {
    fn default() -> Self { Self::new() }
}

impl<T> WorldSerializer for Registry<T>
where
    T: TypeKey,
{
    type TypeId = T;

    unsafe fn serialize_component<S: Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Some((_, serialize_fn, _, _)) = self.serialize_fns.get(&ty) {
            let mut serializer = Some(serializer);
            let mut result = None;
            let result_ref = &mut result;
            (serialize_fn)(ptr, &mut move |serializable| {
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
            .map(|(type_id, _, _, _)| type_id.clone())
    }
}

impl<T> WorldDeserializer for Registry<T>
where
    T: TypeKey,
{
    type TypeId = T;

    fn unmap_id(&self, type_id: &Self::TypeId) -> Option<ComponentTypeId> {
        self.constructors.get(type_id).map(|(id, _)| *id)
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
        if let Some((_, _, deserialize, _)) = self.serialize_fns.get(&type_id) {
            use serde::de::Error;
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            (deserialize)(storage, arch_index, &mut deserializer).map_err(D::Error::custom)
        } else {
            //Err(D::Error::custom("unrecognized component type"))
            panic!()
        }
    }

    fn deserialize_component<'de, D: serde::Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        deserializer: D,
    ) -> Result<Box<[u8]>, D::Error> {
        if let Some((_, _, _, deserialize)) = self.serialize_fns.get(&type_id) {
            use serde::de::Error;
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            (deserialize)(&mut deserializer).map_err(D::Error::custom)
        } else {
            //Err(D::Error::custom("unrecognized component type"))
            panic!()
        }
    }
}

impl<'de, T: TypeKey> DeserializeSeed<'de> for Registry<T> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wrapped = de::Wrapper(self);
        wrapped.deserialize(deserializer)
    }
}

/// A default serializable type ID. This ID is not stable between compiles,
/// so serialized words serialized by a different binary may not deserialize
/// correctly.
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

#[derive(serde::Serialize, serde::Deserialize)]
struct WorldMeta<T> {
    entity_id_offset: u64,
    entity_id_stride: u64,
    entity_id_next: u64,
    component_groups: Vec<Vec<T>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum WorldField {
    _Meta,
    Packed,
    Entities,
}

#[cfg(test)]
mod test {
    use super::Registry;
    use crate::{
        query::filter::any,
        world::{EntityStore, World},
    };

    #[test]
    fn serialize_json() {
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

        let mut registry = Registry::<String>::new();
        registry.register::<usize>("usize".to_string());
        registry.register::<bool>("bool".to_string());
        registry.register::<isize>("isize".to_string());

        let json = serde_json::to_value(&world.as_serializable(any(), &registry)).unwrap();
        println!("{:#}", json);

        use serde::de::DeserializeSeed;
        let world: World = registry.deserialize(json).unwrap();
        let entity = world.entry_ref(entity).unwrap();
        assert_eq!(entity.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entity.get_component::<bool>().unwrap(), &false);
        assert_eq!(entity.get_component::<isize>().unwrap(), &1isize);
    }

    #[test]
    fn serialize_bincode() {
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

        let mut registry = Registry::<i32>::new();
        registry.register::<usize>(1);
        registry.register::<bool>(2);
        registry.register::<isize>(3);

        let encoded = bincode::serialize(&world.as_serializable(any(), &registry)).unwrap();

        use bincode::config::Options;
        use serde::de::DeserializeSeed;
        let mut deserializer = bincode::de::Deserializer::from_slice(
            &encoded[..],
            bincode::config::DefaultOptions::new()
                .with_fixint_encoding()
                .allow_trailing_bytes(),
        );
        let world: World = registry.deserialize(&mut deserializer).unwrap();
        let entity = world.entry_ref(entity).unwrap();
        assert_eq!(entity.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entity.get_component::<bool>().unwrap(), &false);
        assert_eq!(entity.get_component::<isize>().unwrap(), &1isize);
    }
}
