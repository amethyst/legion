use super::{
    hash::U64Hasher,
    storage::{archetype::ArchetypeIndex, ComponentIndex},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Debug,
    hash::BuildHasherDefault,
    sync::atomic::{AtomicU64, Ordering},
};

/// An opaque identifier for an entity.
#[derive(Debug, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Entity(u64);

thread_local! {
    pub static ID_CLONE_MAPPINGS: RefCell<HashMap<Entity, Entity, EntityHasher>> = RefCell::new(HashMap::default());
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        ID_CLONE_MAPPINGS.with(|cell| {
            let map = cell.borrow();
            *map.get(self).unwrap_or(self)
        })
    }
}

const BLOCK_SIZE: u64 = 16;
const BLOCK_SIZE_USIZE: usize = BLOCK_SIZE as usize;

static NEXT_ENTITY: AtomicU64 = AtomicU64::new(0);

/// An iterator which yields new entity IDs.
#[derive(Debug)]
pub struct Allocate {
    base: u64,
    count: u64,
}

impl Allocate {
    /// Constructs a new enity ID allocator iterator.
    pub fn new() -> Self {
        Self { base: 0, count: 0 }
    }
}

impl Default for Allocate {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Iterator for Allocate {
    type Item = Entity;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            self.base = NEXT_ENTITY.fetch_add(BLOCK_SIZE, Ordering::Relaxed);
            self.count = BLOCK_SIZE;
        }

        self.count -= 1;
        Some(Entity(self.base + self.count))
    }
}

/// The storage location of an entity's data.
#[derive(Debug, Copy, Clone)]
pub struct EntityLocation(pub(crate) ArchetypeIndex, pub(crate) ComponentIndex);

impl EntityLocation {
    /// Constructs a new entity location.
    pub fn new(archetype: ArchetypeIndex, component: ComponentIndex) -> Self {
        EntityLocation(archetype, component)
    }

    /// Returns the entity's archetype index.
    pub fn archetype(&self) -> ArchetypeIndex {
        self.0
    }

    /// Returns the entity's component index within its archetype.
    pub fn component(&self) -> ComponentIndex {
        self.1
    }
}

/// A hasher optimized for entity IDs.
pub type EntityHasher = BuildHasherDefault<U64Hasher>;

/// A map of entity IDs to their storage locations.
#[derive(Clone, Default)]
pub struct LocationMap {
    len: usize,
    blocks: HashMap<u64, Box<[Option<EntityLocation>; BLOCK_SIZE_USIZE]>, EntityHasher>,
}

impl Debug for LocationMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.blocks.iter().flat_map(|(base, locs)| {
            locs.iter()
                .enumerate()
                .filter_map(move |(i, loc)| loc.map(|loc| (Entity(*base + i as u64), loc)))
        });
        f.debug_map().entries(entries).finish()
    }
}

impl LocationMap {
    /// Returns the number of entities in the map.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the location map is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the location map contains the given entity.
    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    /// Inserts an collection of adjacent entities into the location map.
    pub fn insert(
        &mut self,
        ids: &[Entity],
        arch: ArchetypeIndex,
        ComponentIndex(base): ComponentIndex,
    ) {
        let mut current_block = u64::MAX;
        let mut block_vec = None;
        for (i, entity) in ids.iter().enumerate() {
            let block = entity.0 / BLOCK_SIZE;
            if current_block != block {
                block_vec = Some(
                    self.blocks
                        .entry(block)
                        .or_insert_with(|| Box::new([None; BLOCK_SIZE_USIZE])),
                );
                current_block = block;
            }

            if let Some(ref mut vec) = block_vec {
                let idx = (entity.0 - block * BLOCK_SIZE) as usize;
                if vec[idx]
                    .replace(EntityLocation(arch, ComponentIndex(base + i)))
                    .is_none()
                {
                    self.len += 1;
                }
            }
        }
    }

    /// Inserts or updates the location of an entity.
    pub fn set(&mut self, entity: Entity, location: EntityLocation) {
        self.insert(&[entity], location.archetype(), location.component());
    }

    /// Returns the location of an entity.
    pub fn get(&self, entity: Entity) -> Option<EntityLocation> {
        let block = entity.0 / BLOCK_SIZE;
        let idx = (entity.0 - block * BLOCK_SIZE) as usize;
        if let Some(&result) = self.blocks.get(&block).and_then(|v| v.get(idx)) {
            result
        } else {
            None
        }
    }

    /// Removes an entity from the location map.
    pub fn remove(&mut self, entity: Entity) -> Option<EntityLocation> {
        let block = entity.0 / BLOCK_SIZE;
        let idx = (entity.0 - block * BLOCK_SIZE) as usize;
        if let Some(loc) = self.blocks.get_mut(&block).and_then(|v| v.get_mut(idx)) {
            let original = *loc;
            if original.is_some() {
                self.len -= 1;
            }
            *loc = None;
            original
        } else {
            None
        }
    }
}
