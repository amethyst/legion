use legion::{
    entity::EntityAllocator,
    prelude::*,
    storage::{
        ArchetypeDescription, ComponentMeta, ComponentResourceSet, ComponentTypeId,
        ComponentWriter, TagMeta, TagStorage, TagTypeId,
    },
};
use serde::{
    de::{DeserializeSeed, IgnoredAny},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{any::TypeId, cell::RefCell, collections::HashMap, ptr::NonNull};
use type_uuid::TypeUuid;

#[derive(TypeUuid, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[uuid = "5fd8256d-db36-4fe2-8211-c7b3446e1927"]
struct Pos(f32, f32, f32);
#[derive(TypeUuid, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[uuid = "14dec17f-ae14-40a3-8e44-e487fc423287"]
struct Vel(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Unregistered(f32, f32, f32);

#[derive(Clone)]
struct TypeRegistration {
    uuid: type_uuid::Bytes,
    ty: TypeId,
    tag_serialize_fn: fn(&TagStorage, &mut dyn FnMut(&dyn erased_serde::Serialize)),
    tag_deserialize_fn: fn(
        deserializer: &mut erased_serde::Deserializer,
        &mut TagStorage,
    ) -> Result<(), erased_serde::Error>,
    comp_serialize_fn: fn(&ComponentResourceSet, &mut dyn FnMut(&dyn erased_serde::Serialize)),
    comp_deserialize_fn: fn(
        deserializer: &mut erased_serde::Deserializer,
        &mut dyn FnMut(NonNull<u8>, usize),
    ) -> Result<(), erased_serde::Error>,
    register_tag_fn: fn(&mut ArchetypeDescription),
    register_comp_fn: fn(&mut ArchetypeDescription),
}
// should separate this into TagRegistration and ComponentRegistration
// Tag type constraints are much stricter, requiring PartialEq and Clone
impl TypeRegistration {
    fn of<
        T: TypeUuid
            + Serialize
            + for<'de> Deserialize<'de>
            + PartialEq
            + Clone
            + Send
            + Sync
            + 'static,
    >() -> Self {
        Self {
            uuid: T::UUID,
            ty: TypeId::of::<T>(),
            tag_serialize_fn: |tag_storage, serialize_fn| {
                // it's safe because we know this is the correct type due to lookup
                let slice = unsafe { tag_storage.data_slice::<T>() };
                serialize_fn(&&*slice);
            },
            tag_deserialize_fn: |deserializer, tag_storage| {
                // TODO implement visitor to avoid allocation of Vec
                let mut tag_vec = <Vec<T> as Deserialize>::deserialize(deserializer)?;
                for tag in tag_vec {
                    // Tag types should line up, making this safe
                    unsafe {
                        tag_storage.push(tag);
                    }
                }
                Ok(())
            },
            comp_serialize_fn: |comp_storage, serialize_fn| {
                // it's safe because we know this is the correct type due to lookup
                let slice = unsafe { comp_storage.data_slice::<T>() };
                serialize_fn(&*slice);
            },
            comp_deserialize_fn: |deserializer, write_components| {
                // TODO implement visitor to avoid allocation of Vec
                let mut comp_vec = <Vec<T> as Deserialize>::deserialize(deserializer)?;
                unsafe {
                    write_components(
                        NonNull::new_unchecked(comp_vec.as_ptr() as *mut T as *mut u8),
                        comp_vec.len(),
                    );
                }
                for comp in comp_vec.drain(0..comp_vec.len()) {
                    std::mem::forget(comp);
                }
                Ok(())
            },
            register_tag_fn: |mut desc| {
                desc.register_tag::<T>();
            },
            register_comp_fn: |mut desc| {
                desc.register_component::<T>();
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SerializedArchetypeDescription {
    tag_types: Vec<type_uuid::Bytes>,
    component_types: Vec<type_uuid::Bytes>,
}

struct SerializeImpl {
    types: HashMap<TypeId, TypeRegistration>,
}
impl legion::ser::WorldSerializer for SerializeImpl {
    fn can_serialize_tag(&self, ty: &TagTypeId, _meta: &TagMeta) -> bool {
        self.types.get(&ty.0).is_some()
    }
    fn can_serialize_component(&self, ty: &ComponentTypeId, _meta: &ComponentMeta) -> bool {
        self.types.get(&ty.0).is_some()
    }
    fn serialize_archetype_description<S: Serializer>(
        &self,
        serializer: S,
        archetype_desc: &ArchetypeDescription,
    ) -> Result<S::Ok, S::Error> {
        let tags_to_serialize = archetype_desc
            .tags()
            .iter()
            .filter_map(|(ty, _)| self.types.get(&ty.0))
            .map(|reg| reg.uuid)
            .collect::<Vec<_>>();
        let components_to_serialize = archetype_desc
            .components()
            .iter()
            .filter_map(|(ty, _)| self.types.get(&ty.0))
            .map(|reg| reg.uuid)
            .collect::<Vec<_>>();
        SerializedArchetypeDescription {
            tag_types: tags_to_serialize,
            component_types: components_to_serialize,
        }
        .serialize(serializer)
    }
    fn serialize_components<S: Serializer>(
        &self,
        serializer: S,
        component_type: &ComponentTypeId,
        _component_meta: &ComponentMeta,
        components: &ComponentResourceSet,
    ) -> Result<S::Ok, S::Error> {
        if let Some(reg) = self.types.get(&component_type.0) {
            let result = RefCell::new(None);
            let serializer = RefCell::new(Some(serializer));
            {
                let mut result_ref = result.borrow_mut();
                (reg.comp_serialize_fn)(components, &mut |serialize| {
                    result_ref.replace(erased_serde::serialize(
                        serialize,
                        serializer.borrow_mut().take().unwrap(),
                    ));
                });
            }
            return result.borrow_mut().take().unwrap();
        }
        panic!(
            "received unserializable type {:?}, this should be filtered by can_serialize",
            component_type
        );
    }
    fn serialize_tags<S: Serializer>(
        &self,
        serializer: S,
        tag_type: &TagTypeId,
        _tag_meta: &TagMeta,
        tags: &TagStorage,
    ) -> Result<S::Ok, S::Error> {
        if let Some(reg) = self.types.get(&tag_type.0) {
            let result = RefCell::new(None);
            let serializer = RefCell::new(Some(serializer));
            {
                let mut result_ref = result.borrow_mut();
                (reg.tag_serialize_fn)(tags, &mut |serialize| {
                    result_ref.replace(erased_serde::serialize(
                        serialize,
                        serializer.borrow_mut().take().unwrap(),
                    ));
                });
            }
            return result.borrow_mut().take().unwrap();
        }
        panic!(
            "received unserializable type {:?}, this should be filtered by can_serialize",
            tag_type
        );
    }
    fn serialize_entities<S: Serializer>(
        &self,
        serializer: S,
        entities: &[Entity],
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(entities.iter().map(|_e| *uuid::Uuid::new_v4().as_bytes()))
    }
}

struct DeserializeImpl {
    types: HashMap<TypeId, TypeRegistration>,
    types_by_uuid: HashMap<type_uuid::Bytes, TypeRegistration>,
    entity_map: RefCell<HashMap<uuid::Bytes, Entity>>,
}
impl legion::de::WorldDeserializer for DeserializeImpl {
    fn deserialize_archetype_description<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
    ) -> Result<ArchetypeDescription, <D as Deserializer<'de>>::Error> {
        let serialized_desc =
            <SerializedArchetypeDescription as Deserialize>::deserialize(deserializer)?;
        let mut desc = ArchetypeDescription::default();
        for tag in serialized_desc.tag_types {
            if let Some(reg) = self.types_by_uuid.get(&tag) {
                (reg.register_tag_fn)(&mut desc);
            }
        }
        for comp in serialized_desc.component_types {
            if let Some(reg) = self.types_by_uuid.get(&comp) {
                (reg.register_comp_fn)(&mut desc);
            }
        }
        Ok(desc)
    }
    fn deserialize_components<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        component_type: &ComponentTypeId,
        component_meta: &ComponentMeta,
        write_component: &mut dyn FnMut(NonNull<u8>, usize),
    ) -> Result<(), <D as Deserializer<'de>>::Error> {
        if let Some(reg) = self.types.get(&component_type.0) {
            let mut erased = erased_serde::Deserializer::erase(deserializer);
            (reg.comp_deserialize_fn)(&mut erased, write_component)
                .map_err(<<D as serde::Deserializer<'de>>::Error as serde::de::Error>::custom)?;
        } else {
            <IgnoredAny>::deserialize(deserializer)?;
        }
        Ok(())
    }
    fn deserialize_tags<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        tag_type: &TagTypeId,
        tag_meta: &TagMeta,
        tags: &mut TagStorage,
    ) -> Result<(), <D as Deserializer<'de>>::Error> {
        if let Some(reg) = self.types.get(&tag_type.0) {
            let mut erased = erased_serde::Deserializer::erase(deserializer);
            println!("deserialized {:?}", tag_type.0);
            (reg.tag_deserialize_fn)(&mut erased, tags)
                .map_err(<<D as serde::Deserializer<'de>>::Error as serde::de::Error>::custom)?;
        } else {
            <IgnoredAny>::deserialize(deserializer)?;
        }
        Ok(())
    }
    fn deserialize_entities<'de, D: Deserializer<'de>>(
        &self,
        deserializer: D,
        entity_allocator: &mut EntityAllocator,
        entities: &mut Vec<Entity>,
    ) -> Result<(), <D as Deserializer<'de>>::Error> {
        let entity_uuids = <Vec<uuid::Bytes> as Deserialize>::deserialize(deserializer)?;
        let mut entity_map = self.entity_map.borrow_mut();
        for id in entity_uuids {
            let entity = entity_allocator.create_entity();
            entity_map.insert(id, entity);
            entities.push(entity);
        }
        Ok(())
    }
}

fn main() {
    // create world
    let universe = Universe::new();
    let mut world = universe.create_world();

    // Pos and Vel are both serializable, so all components in this chunkset will be serialized
    world.insert(
        (),
        vec![
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
        ],
    );
    // Unserializable components are not serialized, so only the Pos components should be serialized in this chunkset
    world.insert(
        (Pos(4., 5., 6.), Unregistered(4., 5., 6.)),
        vec![
            (Pos(1., 2., 3.), Unregistered(4., 5., 6.)),
            (Pos(1., 2., 3.), Unregistered(4., 5., 6.)),
            (Pos(1., 2., 3.), Unregistered(4., 5., 6.)),
            (Pos(1., 2., 3.), Unregistered(4., 5., 6.)),
        ],
    );
    // Entities with no serializable components are not serialized, so this entire chunkset should be skipped in the output
    world.insert(
        (Unregistered(4., 5., 6.),),
        vec![(Unregistered(4., 5., 6.),), (Unregistered(4., 5., 6.),)],
    );

    let registrations = [TypeRegistration::of::<Pos>(), TypeRegistration::of::<Vel>()];

    use std::iter::FromIterator;
    let ser_helper = SerializeImpl {
        types: HashMap::from_iter(registrations.iter().map(|reg| (reg.ty, reg.clone()))),
    };
    let de_helper = DeserializeImpl {
        types: HashMap::from_iter(registrations.iter().map(|reg| (reg.ty, reg.clone()))),
        types_by_uuid: HashMap::from_iter(registrations.iter().map(|reg| (reg.uuid, reg.clone()))),
        entity_map: RefCell::new(HashMap::new()),
    };

    let serializable = legion::ser::serializable_world(&world, &ser_helper);
    let json_data = serde_json::to_string(&serializable).unwrap();
    println!("{}", json_data);
    let mut deserialized_world = universe.create_world();
    let mut deserializer = serde_json::Deserializer::from_str(&json_data);
    legion::de::deserialize(&mut deserialized_world, &de_helper, &mut deserializer).unwrap();
    let serializable = legion::ser::serializable_world(&deserialized_world, &ser_helper);
    let roundtrip_json_data = serde_json::to_string(&serializable).unwrap();
}
