//! Contains types required to serialize and deserialize a world via the serde library.

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
    Entity,
};
use de::{WorldDeserializer, WorldVisitor};
use id::{Canon, EntitySerializer};
use ser::WorldSerializer;
use serde::{de::DeserializeSeed, Serializer};
use std::{collections::HashMap, hash::Hash, marker::PhantomData};

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
/// A [TypeKey](trait.TypeKey.html) which can construct itself for a given type T.
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

/// Describes how to serialize and deserialize a runtime `Entity` ID.
pub trait CustomEntitySerializer {
    type SerializedID: serde::Serialize + for<'a> serde::Deserialize<'a>;
    /// Constructs the serializable representation of `Entity`
    fn to_serialized(&mut self, entity: Entity) -> Self::SerializedID;

    /// Convert a `SerializedEntity` to an `Entity`.
    fn from_serialized(&mut self, serialized: Self::SerializedID) -> Entity;
}

/// A world (de)serializer which describes how to (de)serialize the component types in a world.
///
/// The type parameter `T` represents the key used in the serialized output to identify each
/// component type. The type keys used must uniquely identify each component type, and be stable
/// between recompiles.
///
/// See the [legion_typeuuid crate](https://github.com/TomGillen/legion_typeuuid) for an example
/// of a type key which is stable between compiles.
pub struct Registry<T, S = Canon>
where
    T: TypeKey,
    S: CustomEntitySerializer + 'static,
{
    _phantom_t: PhantomData<T>,
    _phantom_s: PhantomData<S>,
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
    canon: parking_lot::Mutex<S>,
}

impl<T, S> Registry<T, S>
where
    T: TypeKey,
    S: CustomEntitySerializer + 'static,
{
    /// Constructs a new registry.
    pub fn new(entity_serializer: S) -> Self {
        Self {
            missing: UnknownType::Error,
            serialize_fns: HashMap::new(),
            constructors: HashMap::new(),
            canon: parking_lot::Mutex::new(entity_serializer),
            _phantom_t: PhantomData,
            _phantom_s: PhantomData,
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
    pub fn as_deserialize_into_world<'a>(
        &'a self,
        world: &'a mut World,
    ) -> DeserializeIntoWorld<'a, Self> {
        DeserializeIntoWorld(&self, world)
    }

    /// Constructs a serde::DeserializeSeed which will deserialize into a new world.
    pub fn as_deserialize(&self) -> DeserializeNewWorld<'_, Self> {
        DeserializeNewWorld(&self)
    }
}

impl<T, S> Default for Registry<T, S>
where
    T: TypeKey,
    S: CustomEntitySerializer + Default + 'static,
{
    fn default() -> Self {
        Self::new(S::default())
    }
}

impl<S: CustomEntitySerializer + 'static> EntitySerializer for &parking_lot::Mutex<S> {
    fn serialize(
        &self,
        entity: Entity,
        serialize_fn: &mut dyn FnMut(&dyn erased_serde::Serialize),
    ) {
        let mut canon = self.lock();
        let serialized = canon.to_serialized(entity);
        serialize_fn(&serialized);
    }

    fn deserialize(
        &self,
        deserializer: &mut dyn erased_serde::Deserializer,
    ) -> Result<Entity, erased_serde::Error> {
        let mut canon = self.lock();
        let serialized =
            <<S as CustomEntitySerializer>::SerializedID as serde::Deserialize>::deserialize(
                deserializer,
            )?;
        Ok(canon.from_serialized(serialized))
    }
}

impl<T, S> WorldSerializer for Registry<T, S>
where
    T: TypeKey,
    S: CustomEntitySerializer + 'static,
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
    fn with_entity_serializer(&self, callback: &mut dyn FnMut(&dyn EntitySerializer)) {
        let canon_ref = &self.canon;
        callback(&canon_ref);
    }
}

impl<T, S> WorldDeserializer for Registry<T, S>
where
    T: TypeKey,
    S: CustomEntitySerializer + 'static,
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
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
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
            let mut deserializer = erased_serde::Deserializer::erase(deserializer);
            (deserialize)(&mut deserializer).map_err(D::Error::custom)
        } else {
            //Err(D::Error::custom("unrecognized component type"))
            panic!()
        }
    }
    fn with_entity_serializer(&self, callback: &mut dyn FnMut(&dyn EntitySerializer)) {
        let canon_ref = &self.canon;
        callback(&canon_ref);
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

/// Wraps a [WorldDeserializer](de/trait.WorldDeserializer.html) and a world and implements
/// `serde::DeserializeSeed` for deserializing into the world.
pub struct DeserializeIntoWorld<'a, T: WorldDeserializer>(pub &'a T, pub &'a mut World);

impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for DeserializeIntoWorld<'a, W> {
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
pub struct DeserializeNewWorld<'a, T: WorldDeserializer>(pub &'a T);

impl<'a, 'de, W: WorldDeserializer> DeserializeSeed<'de> for DeserializeNewWorld<'a, W> {
    type Value = World;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut world = World::default();
        DeserializeIntoWorld(self.0, &mut world).deserialize(deserializer)?;
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

        let mut registry = Registry::<String>::default();
        registry.register::<usize>("usize".to_string());
        registry.register::<bool>("bool".to_string());
        registry.register::<isize>("isize".to_string());
        registry.register::<EntityRef>("entity_ref".to_string());

        let json = serde_json::to_value(&world.as_serializable(any(), &registry)).unwrap();
        println!("{:#}", json);

        use serde::de::DeserializeSeed;
        let world: World = registry.as_deserialize().deserialize(json).unwrap();
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

        let mut registry = Registry::<i32>::default();
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
            .as_deserialize()
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
            let registry = Registry::<i32>::default();

            super::WorldSerializer::with_entity_serializer(&registry, &mut |canon| {
                super::id::run_as_context(canon, || panic!());
            });
        })
        .unwrap_err();

        // run the serialize_bincode test again
        serialize_bincode();
    }
}
