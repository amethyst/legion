//! Contains types required to serialize and deserialize a world via the serde library.

use super::world::Universe;
use crate::internals::{
    storage::{
        archetype::{ArchetypeIndex, EntityLayout},
        component::{Component, ComponentTypeId},
        UnknownComponentStorage,
    },
    world::World,
};
use de::{WorldDeserializer, WorldVisitor};
use ser::WorldSerializer;
use serde::{de::DeserializeSeed, Serializer};
use std::{collections::HashMap, hash::Hash, marker::PhantomData};

pub mod de;
mod entities;
mod packed;
pub mod ser;

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

/// A [TypeKey](trait.TypeKey.html) which can construct itself for a given type T.
pub trait AutoTypeKey<T: Component>: TypeKey {
    /// Constructs the type key for component type `T`.
    fn new() -> Self;
}

type SerializeFn = fn(*const u8, &mut dyn FnMut(&dyn erased_serde::Serialize));
type DeserializeSingleFn = fn(
    &mut dyn UnknownComponentStorage,
    ArchetypeIndex,
    &mut dyn erased_serde::Deserializer,
) -> Result<(), erased_serde::Error>;
type DeserializeSingleBoxedFn =
    fn(&mut dyn erased_serde::Deserializer) -> Result<Box<[u8]>, erased_serde::Error>;

#[derive(Copy, Clone)]
// An error type describing what to do when a component type is unrecognized.
pub enum UnknownType {
    /// Ignore the component.
    Ignore,
    /// Abort (de)serialization wwith an error.
    Error,
}

/// A world (de)serializer which describes how to (de)serialize the component types in a world.
pub struct Registry<T>
where
    T: TypeKey,
{
    _phantom: PhantomData<T>,
    missing: UnknownType,
    serialize_fns: HashMap<
        ComponentTypeId,
        (
            T,
            SerializeFn,
            DeserializeSingleFn,
            DeserializeSingleBoxedFn,
        ),
    >,
    constructors: HashMap<T, (ComponentTypeId, fn(&mut EntityLayout))>,
}

impl<T> Registry<T>
where
    T: TypeKey,
{
    /// Constructs a new registry.
    pub fn new() -> Self {
        Self {
            missing: UnknownType::Error,
            serialize_fns: HashMap::new(),
            constructors: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Sets the behavior to use when a component type is unknown.
    pub fn on_unknown(&mut self, unknown: UnknownType) {
        self.missing = unknown;
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
        let deserialize_single_fn =
            |storage: &mut dyn UnknownComponentStorage,
             arch: ArchetypeIndex,
             deserializer: &mut dyn erased_serde::Deserializer| {
                let component = erased_serde::deserialize::<C>(deserializer)?;
                unsafe {
                    let ptr = &component as *const C as *const u8;
                    storage.extend_memcopy_raw(arch, ptr, 1);
                    std::mem::forget(component)
                }
                Ok(())
            };
        let deserialize_single_boxed_fn = |deserializer: &mut dyn erased_serde::Deserializer| {
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
                deserialize_single_fn,
                deserialize_single_boxed_fn,
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
        T: AutoTypeKey<C>,
    {
        self.register::<C>(<T as AutoTypeKey<C>>::new())
    }

    /// Constructs a serde::DeserializeSeed which will deserialize into an exist world.
    pub fn as_deserialize_into_world<'a>(
        &'a self,
        world: &'a mut World,
    ) -> WorldDeserializerWrapper<'a, Self> {
        WorldDeserializerWrapper(&self, world)
    }

    /// Constructs a serde::DeserializeSeed which will deserialize into a new world in the given universe.
    pub fn as_deserialize<'a>(
        &'a self,
        universe: &'a Universe,
    ) -> UniverseDeserializerWrapper<'a, Self> {
        UniverseDeserializerWrapper(&self, universe)
    }
}

impl<T: TypeKey> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
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

    fn map_id(&self, type_id: ComponentTypeId) -> Result<Self::TypeId, UnknownType> {
        if let Some(type_id) = self
            .serialize_fns
            .get(&type_id)
            .map(|(type_id, _, _, _)| type_id.clone())
        {
            Ok(type_id)
        } else {
            Err(self.missing)
        }
    }
}

impl<T> WorldDeserializer for Registry<T>
where
    T: TypeKey,
{
    type TypeId = T;

    fn unmap_id(&self, type_id: &Self::TypeId) -> Result<ComponentTypeId, UnknownType> {
        if let Some(type_id) = self.constructors.get(type_id).map(|(id, _)| *id) {
            Ok(type_id)
        } else {
            Err(self.missing)
        }
    }

    fn register_component(&self, type_id: Self::TypeId, layout: &mut EntityLayout) {
        if let Some((_, constructor)) = self.constructors.get(&type_id) {
            (constructor)(layout);
        }
    }

    fn deserialize_insert_component<'de, D: serde::Deserializer<'de>>(
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

/// Wraps a [WorldDeserializer](de/trait.WorldDeserializer.html) and a world and implements
/// `serde::DeserializeSeed` for deserializing into the world.
pub struct WorldDeserializerWrapper<'a, T: WorldDeserializer>(pub &'a T, pub &'a mut World);

impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for WorldDeserializerWrapper<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(WorldVisitor {
            world_deserializer: self.0,
            world: self.1,
        })
    }
}

/// Wraps a [WorldDeserializer](de/trait.WorldDeserializer.html) and a universe and implements
/// `serde::DeserializeSeed` for deserializing into a new world.
pub struct UniverseDeserializerWrapper<'a, T: WorldDeserializer>(pub &'a T, pub &'a Universe);

impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for UniverseDeserializerWrapper<'a, W> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut world = self.1.create_world();
        WorldDeserializerWrapper(self.0, &mut world).deserialize(deserializer)?;
        Ok(world)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum WorldField {
    Packed,
    Entities,
}

#[cfg(test)]
mod test {
    use super::Registry;
    use crate::internals::{
        entity::Entity,
        query::filter::filter_fns::any,
        world::{EntityStore, Universe, World},
    };

    #[test]
    fn serialize_json() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let entity = world.extend(vec![
            (1usize, false, 1isize),
            (2usize, false, 2isize),
            (3usize, false, 3isize),
            (4usize, false, 4isize),
        ])[0];

        #[derive(serde::Serialize, serde::Deserialize)]
        struct EntityRef(Entity);

        let with_ref = world.extend(vec![
            (5usize, 5isize, EntityRef(entity)),
            (6usize, 6isize, EntityRef(entity)),
            (7usize, 7isize, EntityRef(entity)),
            (8usize, 8isize, EntityRef(entity)),
        ])[0];

        let mut registry = Registry::<String>::new();
        registry.register::<usize>("usize".to_string());
        registry.register::<bool>("bool".to_string());
        registry.register::<isize>("isize".to_string());
        registry.register::<EntityRef>("entity_ref".to_string());

        let json = serde_json::to_value(&world.as_serializable(any(), &registry)).unwrap();
        println!("{:#}", json);

        use serde::de::DeserializeSeed;
        let world: World = registry
            .as_deserialize(&universe)
            .deserialize(json)
            .unwrap();
        let entry = world.entry_ref(entity).unwrap();
        assert_eq!(entry.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entry.get_component::<bool>().unwrap(), &false);
        assert_eq!(entry.get_component::<isize>().unwrap(), &1isize);
        assert_eq!(
            world
                .entry_ref(with_ref)
                .unwrap()
                .get_component::<EntityRef>()
                .unwrap()
                .0,
            entity
        );

        assert_eq!(8, world.len());
    }

    #[test]
    fn serialize_bincode() {
        let universe = Universe::new();
        let mut world = universe.create_world();

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
        let world: World = registry
            .as_deserialize(&universe)
            .deserialize(&mut deserializer)
            .unwrap();
        let entity = world.entry_ref(entity).unwrap();
        assert_eq!(entity.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entity.get_component::<bool>().unwrap(), &false);
        assert_eq!(entity.get_component::<isize>().unwrap(), &1isize);

        assert_eq!(8, world.len());
    }
}
