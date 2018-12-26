use std::fmt::Display;
use std::sync::Arc;
use parking_lot::Mutex;
use slog::{Drain, o, debug};

pub type EntityIndex = u16;
pub type EntityVersion = u16;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Entity {
    index: EntityIndex,
    version: EntityVersion
}

impl Entity {
    pub fn none() -> Entity {
        Entity {
            index: 0,
            version: 0
        }
    }

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
    allocator: Arc<Mutex<EntityAllocator>>
}

impl Universe {
    pub fn new<L: Into<Option<slog::Logger>>>(logger: L) -> Self {
        Universe {
            logger: logger.into().unwrap_or(slog::Logger::root(slog_stdlog::StdLog.fuse(), o!())),
            allocator: Arc::from(Mutex::new(EntityAllocator::new()))
        }
    }

    pub fn create_world(&self) -> World {
        debug!(self.logger, "Creating world");
        World::new(self.allocator.clone())
    }
}

struct EntityAllocator {
    allocated: usize,
    free: Vec<EntityBlock>
}

impl EntityAllocator {
    const BLOCK_SIZE: usize = 1024;

    pub fn new() -> Self {
        EntityAllocator {
            allocated: 0,
            free: Vec::new()
        }
    }

    pub fn allocate(&mut self) -> EntityBlock {
        if let Some(block) = self.free.pop() {
            block
        } else {            
            let block = EntityBlock::new(self.allocated as EntityIndex, EntityAllocator::BLOCK_SIZE);
            self.allocated += EntityAllocator::BLOCK_SIZE;
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
            self.versions.push(1);
            Some(Entity::new(index, 1))
        } else {
            None
        }
    }

    pub fn free(&mut self, entity: Entity) -> Option<bool> {
        if let Some(alive) = self.is_alive(&entity) {
            let i = self.index(entity.index);
            self.versions[i] += 1;
            self.free.push(entity.index);
            Some(alive)
        } else {
            None
        }
    }
}

pub struct World {
    allocator: Arc<Mutex<EntityAllocator>>,
    blocks: Vec<EntityBlock>
}

impl World {
    fn new(allocator: Arc<Mutex<EntityAllocator>>) -> Self {
        World {
            allocator: allocator,
            blocks: Vec::new()
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
        if let Some(entity) = self.blocks
            .iter_mut()
            .rev()
            .filter_map(|b| b.allocate())
            .nth(0) {
            entity
        } else {
            let mut block = self.allocator.lock().allocate();
            let entity = block.allocate().unwrap();
            self.blocks.push(block);
            entity
        }
    }

    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        self.blocks
            .iter_mut()
            .filter_map(|b| b.free(entity))
            .nth(0)
            .unwrap_or(false)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        for block in self.blocks.drain(..) {
            self.allocator.lock().free(block);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::collections::HashSet;

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
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        world.create_entity();
    }

    #[test]
    fn create_entity_many() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        for _ in 0..512 {
            world.create_entity();
        }
    }

    #[test]
    fn create_entity_many_blocks() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        for _ in 0..3000 {
            world.create_entity();
        }
    }

    #[test]
    fn create_entity_recreate() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        for _ in 0..3 {
            let entities : Vec<Entity> = (0..512).map(|_| world.create_entity()).collect();
            for e in entities {
                world.delete_entity(e);
            }
        }
    }

    #[test]
    fn is_alive_allocated() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let entity = world.create_entity();

        assert_eq!(true, world.is_alive(&entity));
    }

    #[test]
    fn is_alive_unallocated() {
        let universe = Universe::new(None);
        let world = universe.create_world();
        let entity = Entity::none();

        assert_eq!(false, world.is_alive(&entity));
    }

    #[test]
    fn is_alive_killed() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let entity = world.create_entity();
        world.delete_entity(entity);

        assert_eq!(false, world.is_alive(&entity));
    }

    #[test]
    fn delete_entity_was_alive() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let entity = world.create_entity();
        
        assert_eq!(true, world.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_dead() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let entity = world.create_entity();
        world.delete_entity(entity);
        
        assert_eq!(false, world.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_unallocated() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let entity = Entity::none();
        
        assert_eq!(false, world.delete_entity(entity));
    }

    #[test]
    fn multiple_world_unique_ids() {
        let universe = Universe::new(None);
        let mut world_a = universe.create_world();
        let mut world_b = universe.create_world();

        let mut entities_a = HashSet::<Entity>::new();
        let mut entities_b = HashSet::<Entity>::new();

        for _ in 0..5 {
            entities_a.extend((0..1500).map(|_| world_a.create_entity()));
            entities_b.extend((0..1500).map(|_| world_b.create_entity()));
        }

        assert_eq!(true, entities_a.is_disjoint(&entities_b));

        for e in entities_a {
            assert_eq!(true, world_a.is_alive(&e));
            assert_eq!(false, world_b.is_alive(&e));
        }

        for e in entities_b {
            assert_eq!(false, world_a.is_alive(&e));
            assert_eq!(true, world_b.is_alive(&e));
        }
    }
}
