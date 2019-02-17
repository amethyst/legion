#![feature(specialization)]
#![feature(fnbox)]

use std::boxed::FnBox;
use std::mem::size_of;
use downcast_rs::{Downcast, impl_downcast};
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::any::TypeId;
use std::collections::HashSet;
use std::num::Wrapping;
use std::fmt::Display;
use std::sync::Arc;
use parking_lot::Mutex;
use slog::{Drain, o, debug};

pub type EntityIndex = u16;
pub type EntityVersion = Wrapping<u16>;
pub type ComponentID = u16;
pub type ChunkID = u16;
pub type ArchetypeID = u16;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Entity {
    index: EntityIndex,
    version: EntityVersion
}

impl Entity {
    pub fn new(index: EntityIndex, version: EntityVersion) -> Entity {
        Entity {
            index: index,
            version: version
        }
    }
}

impl Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}#{}", self.index, self.version)
    }
}

pub struct Universe {
    logger: slog::Logger,
    allocator: Arc<Mutex<BlockAllocator>>
}

impl Universe {
    pub fn new<L: Into<Option<slog::Logger>>>(logger: L) -> Self {
        Universe {
            logger: logger.into().unwrap_or(slog::Logger::root(slog_stdlog::StdLog.fuse(), o!())),
            allocator: Arc::from(Mutex::new(BlockAllocator::new()))
        }
    }

    pub fn create_world(&self) -> World {
        debug!(self.logger, "Creating world");
        World::new(EntityAllocator::new(self.allocator.clone()))
    }
}

struct BlockAllocator {
    allocated: usize,
    free: Vec<EntityBlock>
}

impl BlockAllocator {
    const BLOCK_SIZE: usize = 1024;

    pub fn new() -> Self {
        BlockAllocator {
            allocated: 0,
            free: Vec::new()
        }
    }

    pub fn allocate(&mut self) -> EntityBlock {
        if let Some(block) = self.free.pop() {
            block
        } else {            
            let block = EntityBlock::new(self.allocated as EntityIndex, BlockAllocator::BLOCK_SIZE);
            self.allocated += BlockAllocator::BLOCK_SIZE;
            block
        }
    }

    pub fn free(&mut self, block: EntityBlock) {
        self.free.push(block);
    }
}

struct EntityBlock {
    start: EntityIndex,
    len: usize,
    versions: Vec<EntityVersion>,
    free: Vec<EntityIndex>
}

impl EntityBlock {
    pub fn new(start: u16, len: usize) -> EntityBlock {
        EntityBlock {
            start: start,
            len: len,
            versions: Vec::new(),
            free: Vec::new()
        }
    }

    fn index(&self, index: EntityIndex) -> usize {
        (index - self.start) as usize
    }

    pub fn is_alive(&self, entity: &Entity) -> Option<bool> {
        if entity.index >= self.start {
            let i = self.index(entity.index);
            self.versions.get(i).map(|v| *v == entity.version)
        } else {
            None
        }
    }

    pub fn allocate(&mut self) -> Option<Entity> {
        if let Some(index) = self.free.pop() {
            let i = self.index(index);
            Some(Entity::new(index, self.versions[i]))
        } else if self.versions.len() < self.len {
            let index = self.start + self.versions.len() as EntityIndex;
            self.versions.push(Wrapping(1));
            Some(Entity::new(index, Wrapping(1)))
        } else {
            None
        }
    }

    pub fn free(&mut self, entity: Entity) -> Option<bool> {
        if let Some(alive) = self.is_alive(&entity) {
            let i = self.index(entity.index);
            self.versions[i] += Wrapping(1);
            self.free.push(entity.index);
            Some(alive)
        } else {
            None
        }
    }
}

pub struct EntityAllocator {
    allocator: Arc<Mutex<BlockAllocator>>,
    blocks: Vec<EntityBlock>,
    entity_buffer: Vec<Entity>
}

impl EntityAllocator {
    fn new(allocator: Arc<Mutex<BlockAllocator>>) -> Self {
        EntityAllocator {
            allocator: allocator,
            blocks: Vec::new(),
            entity_buffer: Vec::new()
        }
    }

    pub fn is_alive(&self, entity: &Entity) -> bool {
        self.blocks
            .iter()
            .filter_map(|b| b.is_alive(entity))
            .nth(0)
            .unwrap_or(false)
    }

    pub fn create_entity(&mut self) -> Entity {
        let entity = if let Some(entity) = self.blocks
            .iter_mut()
            .rev()
            .filter_map(|b| b.allocate())
            .nth(0)
        {
            entity
        } else {
            let mut block = self.allocator.lock().allocate();
            let entity = block.allocate().unwrap();
            self.blocks.push(block);
            entity
        };

        self.entity_buffer.push(entity.clone());
        entity
    }

    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        self.blocks
            .iter_mut()
            .filter_map(|b| b.free(entity))
            .nth(0)
            .unwrap_or(false)
    }

    pub fn allocation_buffer(&self) -> &[Entity] {
        self.entity_buffer.as_slice()
    }

    pub fn clear_allocation_buffer(&mut self) {
        self.entity_buffer.clear();
    }
}

impl Drop for EntityAllocator {
    fn drop(&mut self) {
        for block in self.blocks.drain(..) {
            self.allocator.lock().free(block);
        }
    }
}

pub struct World {
    allocator: Option<EntityAllocator>,
    archetypes: Vec<Archetype>,
    entities: Option<HashMap<Entity, (ArchetypeID, ChunkID, ComponentID)>>
}

impl World {
    fn new(allocator: EntityAllocator) -> Self {
        World {
            allocator: Some(allocator),
            archetypes: Vec::new(),
            entities: Some(HashMap::new())
        }
    }

    pub fn is_alive(&self, entity: &Entity) -> bool {
        self.allocator.as_ref().unwrap().is_alive(entity)
    }

    pub fn insert_from<S, T>(&mut self, shared: S, components: T) -> &[Entity]
    where S: SharedDataSet,
          T: IntoIterator,
          T::IntoIter: ExactSizeIterator,
          T::Item: ComponentDataSet,
          IterComponentSource<T::IntoIter, T::Item>: ComponentSource
    {
        let source = T::Item::component_source(components.into_iter());
        self.insert(shared, source)
    }

    pub fn insert<S, T>(&mut self, shared: S, mut components: T) -> &[Entity]
    where S: SharedDataSet,
          T: ComponentSource
    {
        let mut allocator = self.allocator.take().unwrap();
        let mut entities = self.entities.take().unwrap();

        allocator.clear_allocation_buffer();

        // find or create archetype
        let (arch_id, archetype) = self.archetype(&shared, &components);

        // insert components into chunks
        while !components.is_empty() {
            // find or create chunk
            let (chunk_id, chunk) = World::free_chunk(archetype, &shared, &components);

            // insert as many components as we can into the chunk
            let allocated = components.write(chunk, &mut allocator);

            // record new entity locations
            let start = unsafe { chunk.entities().len() - allocated };
            let added = unsafe { chunk.component_ids().skip(start) };
            for (e, comp_id) in added {
                entities.insert(e, (arch_id, chunk_id, comp_id));
            }
        }

        self.entities = Some(entities);
        self.allocator = Some(allocator);
        self.allocator.as_ref().unwrap().allocation_buffer()
    }

    pub fn delete(&mut self, entity: Entity) -> bool {
        let deleted = self.allocator.as_mut().unwrap().delete_entity(entity);

        if deleted {
            // lookup entity location
            let ids = self.entities.as_ref().unwrap()
                .get(&entity)
                .map(|(archetype_id, chunk_id, component_id)| (*archetype_id, *chunk_id, *component_id));
            
            // swap remove with last entity in chunk
            let swapped = ids.and_then(|(archetype_id, chunk_id, component_id)| {
                self.archetypes
                    .get_mut(archetype_id as usize)
                    .and_then(|archetype| archetype.chunk_mut(chunk_id))
                    .and_then(|chunk| unsafe { chunk.remove(component_id) })
            });

            // record swapped entity's new location
            if let Some(swapped) = swapped {
                self.entities.as_mut().unwrap()
                    .insert(swapped, ids.unwrap());
            }
        }

        deleted
    }

    pub fn component<T: Component>(&self, entity: Entity) -> Option<&T> {        
        self.entities.as_ref()
            .and_then(|e| e.get(&entity))
            .and_then(|(archetype_id, chunk_id, component_id)| {
                self.archetypes
                    .get(*archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(*chunk_id))
                    .and_then(|chunk| unsafe { chunk.components::<T>() })
                    .and_then(|vec| vec.get(*component_id as usize))
            })
    }

    pub fn component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let entities = &self.entities;
        let archetypes = &self.archetypes;
        entities.as_ref()
            .and_then(|e| e.get(&entity))
            .and_then(|(archetype_id, chunk_id, component_id)| {
                archetypes
                    .get(*archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(*chunk_id))
                    .and_then(|chunk| unsafe { chunk.components_mut::<T>() })
                    .and_then(|vec| vec.get_mut(*component_id as usize))
            })
    }

    pub fn shared<T: SharedComponent>(&self, entity: Entity) -> Option<&T> {
        self.entities.as_ref()
            .and_then(|e| e.get(&entity))
            .and_then(|(archetype_id, chunk_id, _)| {
                self.archetypes
                    .get(*archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(*chunk_id))
                    .and_then(|chunk| unsafe { chunk.shared_component::<T>() })
            })
    }

    fn archetype<S: SharedDataSet, C: ComponentSource>(&mut self, shared: &S, components: &C) -> (ArchetypeID, &mut Archetype) {
        // copy of self to bypass NLL2018 failure
        let self2 = unsafe { &mut *(self as *const _ as *mut Self) };
        
        if let Some((id, archetype)) = self.archetypes
            .iter_mut()
            .enumerate()
            .filter(|(_, a)| components.is_archetype_match(a) && shared.is_archetype_match(a))
            .nth(0)
        {
            // original borrow of self ends here
            return (id as ArchetypeID, archetype);
        }

        // now safe to access self again
        let archetype = Archetype::new(components.types(), shared.types());
        self2.archetypes.push(archetype);
        ((self2.archetypes.len() - 1) as ArchetypeID, self2.archetypes.last_mut().unwrap())
    }

    fn free_chunk<'a, S: SharedDataSet, C: ComponentSource>(archetype: &'a mut Archetype, shared: &S, components: &C) -> (ChunkID, &'a mut Chunk) {
        // copy of archetype reference to bypass NLL2018 failure
        let archetype2 = unsafe { &mut *(archetype as *const _ as *mut Archetype) };

        let free = archetype
            .chunks_with_space_mut()
            .enumerate()
            .filter(|(_, c)| shared.is_chunk_match(c))
            .nth(0);

        match free {
            Some((id, chunk)) => (id as ChunkID, chunk), // original borrow ends here
            None => {
                // now safe to access archetype again                
                let mut builder = ChunkBuilder::new();
                shared.configure_chunk(&mut builder);
                components.configure_chunk(&mut builder);

                let chunk = builder.build();                
                archetype2.chunks.push(chunk);
                ((archetype2.chunks.len() - 1) as ChunkID, archetype2.chunks.last_mut().unwrap())
            }
        }
    }
}

pub trait SharedDataSet {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool;
    fn is_chunk_match(&self, chunk: &Chunk) -> bool;
    fn configure_chunk(&self, chunk: &mut ChunkBuilder);
    fn types(&self) -> HashSet<TypeId>;
}

pub trait ComponentDataSet: Sized {
    fn component_source<T>(source: T) -> IterComponentSource<T, Self>
        where T: ExactSizeIterator + Iterator<Item=Self>;
}

pub trait ComponentSource {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool;
    fn configure_chunk(&self, chunk: &mut ChunkBuilder);
    fn types(&self) -> HashSet<TypeId>;
    fn is_empty(&self) -> bool;
    fn write<'a>(&mut self, chunk: &'a mut Chunk, allocator: &mut EntityAllocator) -> usize;
}

impl SharedDataSet for () {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool {
        archetype.shared.len() == 0
    }

    fn is_chunk_match(&self, _: &Chunk) -> bool {
        true
    }

    fn configure_chunk(&self, _: &mut ChunkBuilder) { }

    fn types(&self) -> HashSet<TypeId> {
        HashSet::new()
    }
}

macro_rules! impl_shared_data_set {
    ( $arity: expr; $( $ty: ident ),* ) => {
        impl<$( $ty ),*> SharedDataSet for ($( $ty, )*)
        where $( $ty: SharedComponent ),*
        {
            fn is_archetype_match(&self, archetype: &Archetype) -> bool {
                archetype.shared.len() == $arity &&
                $( archetype.shared.contains(&TypeId::of::<$ty>()) )&&*
            }

            fn is_chunk_match(&self, chunk: &Chunk) -> bool {
                unsafe {
                    #![allow(non_snake_case)]
                    let ($($ty,)*) = self;
                    $(
                        (*chunk.shared_component::<$ty>().unwrap() == *$ty)
                    )&&*
                }
            }

            fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
                #![allow(non_snake_case)]
                let ($( ref $ty, )*) = self;
                $( chunk.register_shared($ty.clone()); )*
            }

            fn types(&self) -> HashSet<TypeId> {
                [$( TypeId::of::<$ty>() ),*].iter().cloned().collect()
            }
        }
    }
}

impl_shared_data_set!(1; A);
impl_shared_data_set!(2; A, B);
impl_shared_data_set!(3; A, B, C);
impl_shared_data_set!(4; A, B, C, D);
impl_shared_data_set!(5; A, B, C, D, E);

pub struct IterComponentSource<T: ExactSizeIterator + Iterator<Item=K>, K> {
    source: T
}

macro_rules! impl_component_source {
    ( $arity: expr; $( $ty: ident => $id: ident ),* ) => {
        impl<$( $ty ),*> ComponentDataSet for ($( $ty, )*)
        where $( $ty: Component ),*
        {
            fn component_source<T>(source: T) -> IterComponentSource<T, Self>
                where T: ExactSizeIterator + Iterator<Item=Self>
            {
                IterComponentSource::<T, Self> { source }
            }
        }

        impl<I, $( $ty ),*> ComponentSource for IterComponentSource<I, ($( $ty, )*)> 
        where I: Iterator<Item=($( $ty, )*)> + ExactSizeIterator,
              $( $ty: Component ),*
        {
            fn types(&self) -> HashSet<TypeId> {
                [$( TypeId::of::<$ty>() ),*].iter().cloned().collect()
            }

            fn is_archetype_match(&self, archetype: &Archetype) -> bool {
                archetype.components.len() == $arity &&
                $(
                    archetype.components.contains(&TypeId::of::<$ty>())
                )&&*
            }

            fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
                $(
                    chunk.register_component::<$ty>();
                )*
            }

            fn is_empty(&self) -> bool {
                self.source.len() == 0
            }
            
            fn write<'a>(&mut self, chunk: &'a mut Chunk, allocator: &mut EntityAllocator) -> usize {
                #![allow(non_snake_case)]
                let mut count = 0;

                unsafe {
                    let entities = chunk.entities_mut();
                    $(
                        let $ty = chunk.components_mut::<$ty>().unwrap();
                    )*
                                     
                    while let Some(($( $id, )*)) = { if chunk.is_full() { None } else { self.source.next() } } {
                        let entity = allocator.create_entity();
                        entities.push(entity);
                        $(
                            $ty.push($id);
                        )*
                        count += 1;
                    }
                }

                chunk.validate();
                count
            }
        }
    }
}

impl_component_source!(1; A => a);
impl_component_source!(2; A => a, B => b);
impl_component_source!(3; A => a, B => b, C => c);
impl_component_source!(4; A => a, B => b, C => c, D => d);
impl_component_source!(5; A => a, B => b, C => c, D => d, E => e);

pub trait Component: Send + Sized + 'static { }
pub trait SharedComponent: Send + Sized + PartialEq + Clone + Sync + 'static { }

impl<T: Send + Sized + 'static> Component for T {}
impl<T: Send + Sized + PartialEq + Clone + Sync + 'static> SharedComponent for T {}

impl_downcast!(ComponentStorage);
pub trait ComponentStorage: Downcast {
    fn remove(&mut self, id: ComponentID);
    fn len(&self) -> usize;
}

pub struct UnsafeVec<T>(UnsafeCell<Vec<T>>);

impl<T> UnsafeVec<T> {
    fn with_capacity(capacity: usize) -> Self {
        UnsafeVec(UnsafeCell::new(Vec::<T>::with_capacity(capacity)))
    }

    unsafe fn inner(&self) -> &Vec<T> {
        &(*self.0.get())
    }

    unsafe fn inner_mut(&self) -> &mut Vec<T> {
        &mut (*self.0.get())
    }
}

impl<T: 'static> ComponentStorage for UnsafeVec<T> {
    fn remove(&mut self, id: ComponentID) {
        unsafe {
            self.inner_mut().swap_remove(id as usize);
        }
    }

    fn len(&self) -> usize {
        unsafe {
            self.inner_mut().len()
        }
    }
}

impl_downcast!(SharedComponentStorage);
trait SharedComponentStorage: Downcast { }

struct SharedComponentStore<T>(UnsafeCell<T>);

impl<T: SharedComponent> SharedComponentStorage for SharedComponentStore<T> {

}

pub struct Chunk {
    len: usize,
    capacity: usize,
    entities: UnsafeVec<Entity>,
    components: HashMap<TypeId, Box<dyn ComponentStorage>>,
    shared: HashMap<TypeId, Arc<dyn SharedComponentStorage>>
}

impl Chunk {
    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    pub unsafe fn entities(&self) -> &Vec<Entity> {
        self.entities.inner()
    }

    pub unsafe fn entities_mut(&self) -> &mut Vec<Entity> {
        self.entities.inner_mut()
    }

    pub unsafe fn component_ids<'a>(&'a self) -> impl Iterator<Item=(Entity, ComponentID)> + ExactSizeIterator + 'a {
        self.entities.inner().iter().enumerate().map(|(i, e)| (*e, i as ComponentID))
    }

    pub unsafe fn components<T: Component>(&self) -> Option<&Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<UnsafeVec<T>>())
            .map(|c| c.inner())
    }

    pub unsafe fn components_mut<T: Component>(&self) -> Option<&mut Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<UnsafeVec<T>>())
            .map(|c| c.inner_mut())
    }

    pub unsafe fn shared_component<T: SharedComponent>(&self) -> Option<&T> {
        self.shared
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<SharedComponentStore<T>>())
            .map(|s| &*s.0.get())
    }

    pub unsafe fn remove(&mut self, id: ComponentID) -> Option<Entity> {
        let index = id as usize;
        self.entities.inner_mut().swap_remove(index);
        for storage in self.components.values_mut() {
            storage.remove(id);
        }

        if self.entities.len() > index {
            Some(*self.entities.inner().get(index).unwrap())
        } else {
            None
        }            
    }

    pub fn validate(&self) {
        let valid = self.components
            .values()
            .fold(true, |total, s| total && s.len() == self.entities.len());
        if !valid {
            panic!("imbalanced chunk components");
        }
    }
}

pub struct ChunkBuilder {
    components: Vec<(TypeId, usize, Box<dyn FnBox(usize) -> Box<dyn ComponentStorage>>)>,
    shared: HashMap<TypeId, Arc<dyn SharedComponentStorage>>
}

impl ChunkBuilder {
    const MAX_SIZE: usize = 16 * 1024;

    pub fn new() -> ChunkBuilder {
        ChunkBuilder {
            components: Vec::new(),
            shared: HashMap::new()
        }
    }

    pub fn register_component<T: Component>(&mut self) {
        let constructor = |capacity| Box::new(UnsafeVec::<T>::with_capacity(capacity)) as Box<dyn ComponentStorage>;
        self.components.push((TypeId::of::<T>(), size_of::<T>(), Box::new(constructor)));
    }

    pub fn register_shared<T: SharedComponent>(&mut self, data: T) {
        self.shared.insert(TypeId::of::<T>(), Arc::new(SharedComponentStore(UnsafeCell::new(data))) as Arc<dyn SharedComponentStorage>);
    }

    pub fn build(self) -> Chunk {
        let size_bytes = *self.components.iter().map(|(_, size, _)| size).max().unwrap_or(&ChunkBuilder::MAX_SIZE);
        let capacity = std::cmp::max(1, ChunkBuilder::MAX_SIZE / size_bytes);        
        Chunk {
            len: 0,
            capacity: capacity,
            entities: UnsafeVec::with_capacity(capacity),
            components: self.components.into_iter().map(|(id, _, con)| (id, con(capacity))).collect(),
            shared: self.shared
        }
    }
}

pub struct Archetype {
    components: HashSet<TypeId>,
    shared: HashSet<TypeId>,
    chunks: Vec<Chunk>
}

impl Archetype {
    pub fn new(components: HashSet<TypeId>, shared: HashSet<TypeId>) -> Archetype {
        Archetype {
            components: components,
            shared: shared,
            chunks: Vec::new()
        }
    }

    pub fn chunk(&self, id: ChunkID) -> Option<&Chunk> {
        self.chunks.get(id as usize)
    }

    pub fn chunk_mut(&mut self, id: ChunkID) -> Option<&mut Chunk> {
        self.chunks.get_mut(id as usize)
    }

    pub fn has_component<T: Component>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    pub fn has_shared<T: SharedComponent>(&self) -> bool {
        self.shared.contains(&TypeId::of::<T>())
    }

    fn chunks_with_space_mut(&mut self) -> impl Iterator<Item=&mut Chunk> {
        self.chunks.iter_mut().filter(|c| !c.is_full())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::collections::HashSet;

    #[derive(Clone, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Debug, PartialEq)]
    struct Rot(f32, f32, f32);
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    struct Model(u32);
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    struct Static;

    #[test]
    fn create_universe() {
        Universe::new(None);
    }

    #[test]
    fn create_world() {
        let universe = Universe::new(None);
        universe.create_world();
    }

    #[test]
    fn create_entity() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        allocator.create_entity();
    }

    #[test]
    fn create_entity_many() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..512 {
            allocator.create_entity();
        }
    }

    #[test]
    fn create_entity_many_blocks() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..3000 {
            allocator.create_entity();
        }
    }

    #[test]
    fn create_entity_recreate() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..3 {
            let entities : Vec<Entity> = (0..512).map(|_| allocator.create_entity()).collect();
            for e in entities {
                allocator.delete_entity(e);
            }
        }
    }

    #[test]
    fn is_alive_allocated() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();

        assert_eq!(true, allocator.is_alive(&entity));
    }

    #[test]
    fn is_alive_unallocated() {
        let allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = Entity::new(10 as EntityIndex, Wrapping(10));

        assert_eq!(false, allocator.is_alive(&entity));
    }

    #[test]
    fn is_alive_killed() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();
        allocator.delete_entity(entity);

        assert_eq!(false, allocator.is_alive(&entity));
    }

    #[test]
    fn delete_entity_was_alive() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();
        
        assert_eq!(true, allocator.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_dead() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();
        allocator.delete_entity(entity);
        
        assert_eq!(false, allocator.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_unallocated() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = Entity::new(10 as EntityIndex, Wrapping(10));
        
        assert_eq!(false, allocator.delete_entity(entity));
    }

    #[test]
    fn multiple_allocators_unique_ids() {
        let blocks = Arc::from(Mutex::new(BlockAllocator::new()));
        let mut allocator_a = EntityAllocator::new(blocks.clone());
        let mut allocator_b = EntityAllocator::new(blocks.clone());

        let mut entities_a = HashSet::<Entity>::new();
        let mut entities_b = HashSet::<Entity>::new();

        for _ in 0..5 {
            entities_a.extend((0..1500).map(|_| allocator_a.create_entity()));
            entities_b.extend((0..1500).map(|_| allocator_b.create_entity()));
        }

        assert_eq!(true, entities_a.is_disjoint(&entities_b));

        for e in entities_a {
            assert_eq!(true, allocator_a.is_alive(&e));
            assert_eq!(false, allocator_b.is_alive(&e));
        }

        for e in entities_b {
            assert_eq!(false, allocator_a.is_alive(&e));
            assert_eq!(true, allocator_b.is_alive(&e));
        }
    }

    #[test]
    fn insert() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (1usize, 2f32, 3u16);
        let components = vec![(4f32, 5u64, 6u16), (4f32, 5u64, 6u16)];
        let entities = world.insert_from(shared, components);

        assert_eq!(2, entities.len());
    }

    #[test]
    fn get_component() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut entities: Vec<Entity> = Vec::new();
        for e in world.insert_from(shared, components.clone()) {
            entities.push(*e);
        }

        for (i, e) in entities.iter().enumerate() {
            assert_eq!(components.get(i).map(|(x, _)| x), world.component(*e));
            assert_eq!(components.get(i).map(|(_, x)| x), world.component(*e));
        }
    }

    #[test]
    fn get_component_empty_world() {
        let universe = Universe::new(None);
        let world = universe.create_world();

        assert_eq!(None, world.component::<i32>(Entity::new(0, Wrapping(0))));
    }

    #[test]
    fn get_component_wrong_type() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let entity = *world.insert_from((), vec![(0f64,)]).get(0).unwrap();

        assert_eq!(None, world.component::<i32>(entity));
    }

    #[test]
    fn get_shared() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut entities: Vec<Entity> = Vec::new();
        for e in world.insert_from(shared, components.clone()) {
            entities.push(*e);
        }

        for e in entities.iter() {
            assert_eq!(Some(&Static), world.shared(*e));
            assert_eq!(Some(&Model(5)), world.shared(*e));
        }
    }

    #[test]
    fn get_shared_empty_world() {
        let universe = Universe::new(None);
        let world = universe.create_world();

        assert_eq!(None, world.shared::<i32>(Entity::new(0, Wrapping(0))));
    }

    #[test]
    fn get_shared_wrong_type() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let entity = *world.insert_from((Static,), vec![(0f64,)]).get(0).unwrap();

        assert_eq!(None, world.shared::<Model>(entity));
    }

    #[test]
    fn delete() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut entities: Vec<Entity> = Vec::new();
        for e in world.insert_from(shared, components.clone()) {
            entities.push(*e);
        }

        for e in entities.iter() {
            assert_eq!(true, world.is_alive(e));
        }

        for e in entities.iter() {
            world.delete(*e);
            assert_eq!(false, world.is_alive(e));
        }
    }

    #[test]
    fn delete_last() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut entities: Vec<Entity> = Vec::new();
        for e in world.insert_from(shared, components.clone()) {
            entities.push(*e);
        }

        let last = *entities.last().unwrap();
        world.delete(last);
        assert_eq!(false, world.is_alive(&last));

        for (i, e) in entities.iter().take(entities.len() - 1).enumerate() {
            assert_eq!(true, world.is_alive(e));
            assert_eq!(components.get(i).map(|(_, x)| x), world.component(*e));
            assert_eq!(components.get(i).map(|(x, _)| x), world.component(*e));
        }
    }

    #[test]
    fn delete_first() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut entities: Vec<Entity> = Vec::new();
        for e in world.insert_from(shared, components.clone()) {
            entities.push(*e);
        }

        let first = *entities.first().unwrap();

        world.delete(first);
        assert_eq!(false, world.is_alive(&first));

        for (i, e) in entities.iter().skip(1).enumerate() {
            assert_eq!(true, world.is_alive(e));
            assert_eq!(components.get(i + 1).map(|(_, x)| x), world.component(*e));
            assert_eq!(components.get(i + 1).map(|(x, _)| x), world.component(*e));
        }
    }
}
