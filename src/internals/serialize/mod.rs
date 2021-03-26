//! Contains types required to serialize and deserialize a world via the serde library.

use std::{collections::HashMap, hash::Hash, marker::PhantomData};

use de::{WorldDeserializer, WorldVisitor};
use id::EntitySerializer;
use ser::WorldSerializer;
use serde::{de::DeserializeSeed, Serializer};

use crate::{
    internals::{
        storage::{
            archetype::{ArchetypeIndex, EntityLayout},
            component::{Component, ComponentTypeId},
            UnknownComponentStorage,
        },
        world::World,
    },
    storage::UnknownComponentWriter,
};

pub mod archetypes;
pub mod de;
mod entities;
pub mod id;
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
/// A [`TypeKey`] which can construct itself for a given type T.
pub trait AutoTypeKey<T: Component>: TypeKey {
    /// Constructs the type key for component type `T`.
    fn new() -> Self;
}

type SerializeFn = fn(*const u8, &mut dyn FnMut(&dyn erased_serde::Serialize));
type SerializeSliceFn =
    fn(&dyn UnknownComponentStorage, ArchetypeIndex, &mut dyn FnMut(&dyn erased_serde::Serialize));
type DeserializeSliceFn = fn(
    UnknownComponentWriter,
    &mut dyn erased_serde::Deserializer,
) -> Result<(), erased_serde::Error>;
type DeserializeSingleBoxedFn =
    fn(&mut dyn erased_serde::Deserializer) -> Result<Box<[u8]>, erased_serde::Error>;

#[derive(Copy, Clone)]
/// An error type describing what to do when a component type is unrecognized.
pub enum UnknownType {
    /// Ignore the component.
    Ignore,
    /// Abort (de)serialization wwith an error.
    Error,
}

/// A world (de)serializer which describes how to (de)serialize the component types in a world.
///
/// The type parameter `T` represents the key used in the serialized output to identify each
/// component type. The type keys used must uniquely identify each component type, and be stable
/// between recompiles.
///
/// See the [legion_typeuuid crate](https://github.com/TomGillen/legion_typeuuid) for an example
/// of a type key which is stable between compiles.
pub struct Registry<T>
where
    T: TypeKey,
{
    _phantom_t: PhantomData<T>,
    missing: UnknownType,
    serialize_fns: HashMap<
        ComponentTypeId,
        (
            T,
            SerializeSliceFn,
            SerializeFn,
            DeserializeSliceFn,
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
            _phantom_t: PhantomData,
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
        let serialize_slice_fn =
            |storage: &dyn UnknownComponentStorage,
             archetype,
             serialize: &mut dyn FnMut(&dyn erased_serde::Serialize)| unsafe {
                let (ptr, len) = storage.get_raw(archetype).unwrap();
                let slice = std::slice::from_raw_parts(ptr as *const C, len);
                (serialize)(&slice);
            };
        let serialize_fn = |ptr, serialize: &mut dyn FnMut(&dyn erased_serde::Serialize)| {
            let component = unsafe { &*(ptr as *const C) };
            (serialize)(component);
        };
        let deserialize_slice_fn =
            |storage: UnknownComponentWriter, deserializer: &mut dyn erased_serde::Deserializer| {
                // todo avoid temp vec
                ComponentSeq::<C> {
                    storage,
                    _phantom: PhantomData,
                }
                .deserialize(deserializer)?;
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
                serialize_slice_fn,
                serialize_fn,
                deserialize_slice_fn,
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

    /// Constructs a serde::DeserializeSeed which will deserialize into an existing world.
    pub fn as_deserialize_into_world<'a, E: EntitySerializer>(
        &'a self,
        world: &'a mut World,
        entity_serializer: &'a E,
    ) -> DeserializeIntoWorld<'a, Self, E> {
        DeserializeIntoWorld {
            world,
            world_deserializer: self,
            entity_serializer,
        }
    }

    /// Constructs a serde::DeserializeSeed which will deserialize into a new world.
    pub fn as_deserialize<'a, E: EntitySerializer>(
        &'a self,
        entity_serializer: &'a E,
    ) -> DeserializeNewWorld<'a, Self, E> {
        DeserializeNewWorld {
            world_deserializer: self,
            entity_serializer,
        }
    }
}

impl<T> Default for Registry<T>
where
    T: TypeKey,
{
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> WorldSerializer for Registry<T>
where
    T: TypeKey,
{
    type TypeId = T;

    unsafe fn serialize_component<Ser: Serializer>(
        &self,
        ty: ComponentTypeId,
        ptr: *const u8,
        serializer: Ser,
    ) -> Result<Ser::Ok, Ser::Error> {
        if let Some((_, _, serialize_fn, _, _)) = self.serialize_fns.get(&ty) {
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
            .map(|(type_id, _, _, _, _)| type_id.clone())
        {
            Ok(type_id)
        } else {
            Err(self.missing)
        }
    }

    unsafe fn serialize_component_slice<Ser: Serializer>(
        &self,
        ty: ComponentTypeId,
        storage: &dyn UnknownComponentStorage,
        archetype: ArchetypeIndex,
        serializer: Ser,
    ) -> Result<Ser::Ok, Ser::Error> {
        if let Some((_, serialize_fn, _, _, _)) = self.serialize_fns.get(&ty) {
            let mut serializer = Some(serializer);
            let mut result = None;
            let result_ref = &mut result;
            (serialize_fn)(storage, archetype, &mut move |serializable| {
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

    fn deserialize_component_slice<'a, 'de, D: serde::Deserializer<'de>>(
        &self,
        type_id: ComponentTypeId,
        storage: UnknownComponentWriter<'a>,
        deserializer: D,
    ) -> Result<(), D::Error> {
        if let Some((_, _, _, deserialize, _)) = self.serialize_fns.get(&type_id) {
            use serde::de::Error;
            let mut deserializer = <dyn erased_serde::Deserializer>::erase(deserializer);
            (deserialize)(storage, &mut deserializer).map_err(D::Error::custom)
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
        if let Some((_, _, _, _, deserialize)) = self.serialize_fns.get(&type_id) {
            use serde::de::Error;
            let mut deserializer = <dyn erased_serde::Deserializer>::erase(deserializer);
            (deserialize)(&mut deserializer).map_err(D::Error::custom)
        } else {
            //Err(D::Error::custom("unrecognized component type"))
            panic!()
        }
    }
}

struct ComponentSeq<'a, T: Component> {
    storage: UnknownComponentWriter<'a>,
    _phantom: PhantomData<T>,
}

impl<'a, 'de, T: Component + for<'b> serde::de::Deserialize<'b>> serde::de::DeserializeSeed<'de>
    for ComponentSeq<'a, T>
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct SeqVisitor<'b, C: Component + for<'c> serde::de::Deserialize<'c>> {
            storage: UnknownComponentWriter<'b>,
            _phantom: PhantomData<C>,
        }

        impl<'b, 'de, C: Component + for<'c> serde::de::Deserialize<'c>> serde::de::Visitor<'de>
            for SeqVisitor<'b, C>
        {
            type Value = ();

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("component seq")
            }

            fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: serde::de::SeqAccess<'de>,
            {
                if let Some(len) = seq.size_hint() {
                    self.storage.ensure_capacity(len);
                }

                while let Some(component) = seq.next_element::<C>()? {
                    unsafe {
                        let ptr = &component as *const C as *const u8;
                        self.storage.extend_memcopy_raw(ptr, 1);
                        std::mem::forget(component)
                    }
                }
                Ok(())
            }
        }

        deserializer.deserialize_seq(SeqVisitor::<T> {
            storage: self.storage,
            _phantom: PhantomData,
        })
    }
}

/// Wraps a [`WorldDeserializer`] and a world and implements `serde::DeserializeSeed`
/// for deserializing into the world.
pub struct DeserializeIntoWorld<'a, W: WorldDeserializer, E: EntitySerializer> {
    /// The [World](../world/struct.World.html) to deserialize into.
    pub world: &'a mut World,
    /// The [WorldDeserializer](trait.WorldDeserializer.html) to use.
    pub world_deserializer: &'a W,
    /// The [EntitySerializer](trait.EntitySerializer.html) to use.
    pub entity_serializer: &'a E,
}

impl<'a, 'de, W: WorldDeserializer, E: EntitySerializer> DeserializeSeed<'de>
    for DeserializeIntoWorld<'a, W, E>
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(WorldVisitor {
            world: self.world,
            world_deserializer: self.world_deserializer,
            entity_serializer: self.entity_serializer,
        })
    }
}

/// Wraps a [`WorldDeserializer`] and a world and implements `serde::DeserializeSeed`
/// for deserializing into a new world.
pub struct DeserializeNewWorld<'a, W: WorldDeserializer, E: EntitySerializer> {
    /// The [WorldDeserializer](trait.WorldDeserializer.html) to use.
    pub world_deserializer: &'a W,
    /// The [EntitySerializer](trait.EntitySerializer.html) to use.
    pub entity_serializer: &'a E,
}

impl<'a, 'de, W: WorldDeserializer, E: EntitySerializer> DeserializeSeed<'de>
    for DeserializeNewWorld<'a, W, E>
{
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut world = World::default();
        DeserializeIntoWorld {
            world: &mut world,
            world_deserializer: self.world_deserializer,
            entity_serializer: self.entity_serializer,
        }
        .deserialize(deserializer)?;
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
        serialize::id::{set_entity_serializer, Canon},
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

        #[derive(serde::Serialize, serde::Deserialize)]
        struct EntityRef(Entity);

        let with_ref = world.extend(vec![
            (5usize, 5isize, EntityRef(entity)),
            (6usize, 6isize, EntityRef(entity)),
            (7usize, 7isize, EntityRef(entity)),
            (8usize, 8isize, EntityRef(entity)),
        ])[0];

        let entity_serializer = Canon::default();
        let mut registry = Registry::<String>::default();
        registry.register::<usize>("usize".to_string());
        registry.register::<bool>("bool".to_string());
        registry.register::<isize>("isize".to_string());
        registry.register::<EntityRef>("entity_ref".to_string());

        let json =
            serde_json::to_value(&world.as_serializable(any(), &registry, &entity_serializer))
                .unwrap();
        println!("{:#}", json);

        use serde::de::DeserializeSeed;
        let world: World = registry
            .as_deserialize(&entity_serializer)
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

        let entity_serializer = Canon::default();
        let mut registry = Registry::<i32>::default();
        registry.register::<usize>(1);
        registry.register::<bool>(2);
        registry.register::<isize>(3);

        let encoded =
            bincode::serialize(&world.as_serializable(any(), &registry, &entity_serializer))
                .unwrap();

        use bincode::config::Options;
        use serde::de::DeserializeSeed;
        let mut deserializer = bincode::de::Deserializer::from_slice(
            &encoded[..],
            bincode::config::DefaultOptions::new()
                .with_fixint_encoding()
                .allow_trailing_bytes(),
        );
        let world: World = registry
            .as_deserialize(&entity_serializer)
            .deserialize(&mut deserializer)
            .unwrap();
        let entity = world.entry_ref(entity).unwrap();
        assert_eq!(entity.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entity.get_component::<bool>().unwrap(), &false);
        assert_eq!(entity.get_component::<isize>().unwrap(), &1isize);

        assert_eq!(8, world.len());
    }

    #[test]
    fn run_as_context_panic() {
        std::panic::catch_unwind(|| {
            let entity_serializer = Canon::default();
            set_entity_serializer(&entity_serializer, || panic!());
        })
        .unwrap_err();

        // run the serialize_bincode test again
        serialize_bincode();
    }

    #[test]
    fn serialize_json_external_canon() {
        let mut world1 = World::default();

        let entity = world1.extend(vec![
            (1usize, false, 1isize),
            (2usize, false, 2isize),
            (3usize, false, 3isize),
            (4usize, false, 4isize),
        ])[0];

        #[derive(serde::Serialize, serde::Deserialize)]
        struct EntityRef(Entity);

        let with_ref = world1.extend(vec![
            (5usize, 5isize, EntityRef(entity)),
            (6usize, 6isize, EntityRef(entity)),
            (7usize, 7isize, EntityRef(entity)),
            (8usize, 8isize, EntityRef(entity)),
        ])[0];

        let entity_serializer = Canon::default();
        let mut registry = Registry::<String>::new();
        registry.register::<usize>("usize".to_string());
        registry.register::<bool>("bool".to_string());
        registry.register::<isize>("isize".to_string());
        registry.register::<EntityRef>("entity_ref".to_string());

        let json =
            serde_json::to_value(&world1.as_serializable(any(), &registry, &entity_serializer))
                .unwrap();
        println!("{:#}", json);

        let entityname = entity_serializer.get_name(entity).unwrap();
        assert_eq!(entity_serializer.get_id(&entityname).unwrap(), entity);

        use serde::de::DeserializeSeed;
        let world2: World = registry
            .as_deserialize(&entity_serializer)
            .deserialize(json)
            .unwrap();
        let entry = world2.entry_ref(entity).unwrap();
        assert_eq!(entry.get_component::<usize>().unwrap(), &1usize);
        assert_eq!(entry.get_component::<bool>().unwrap(), &false);
        assert_eq!(entry.get_component::<isize>().unwrap(), &1isize);
        assert_eq!(
            world2
                .entry_ref(with_ref)
                .unwrap()
                .get_component::<EntityRef>()
                .unwrap()
                .0,
            entity
        );

        assert_eq!(8, world2.len());
    }
}
