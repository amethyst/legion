use super::{
    hash::U64Hasher,
    storage::{ArchetypeIndex, ComponentIndex},
};
use std::{
    collections::HashMap,
    fmt::Debug,
    hash::BuildHasherDefault,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

const BLOCK_SIZE: usize = 64;
const BLOCK_SIZE_U64: u64 = BLOCK_SIZE as u64;

/// Unique identifier for an entity in a world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct Entity(u64);

/// The storage location of an entity's data.
#[derive(Debug, Copy, Clone)]
pub struct EntityLocation(pub(crate) ArchetypeIndex, pub(crate) ComponentIndex);

impl EntityLocation {
    /// Constructs a new entity location.
    pub fn new(archetype: ArchetypeIndex, component: ComponentIndex) -> Self {
        EntityLocation(archetype, component)
    }

    /// Returns the entity's archetype index.
    pub fn archetype(&self) -> ArchetypeIndex { self.0 }

    /// Returns the entity's component index within its archetype.
    pub fn component(&self) -> ComponentIndex { self.1 }
}

/// A hasher optimized for entity IDs.
pub type EntityHasher = BuildHasherDefault<U64Hasher>;

/// A map of entity IDs to their storage locations.
#[derive(Clone, Default)]
pub struct LocationMap {
    len: usize,
    blocks: HashMap<u64, Box<[Option<EntityLocation>; BLOCK_SIZE]>, EntityHasher>,
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
    pub fn len(&self) -> usize { self.len }

    /// Returns `true` if the location map is empty.
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Returns `true` if the location map contains the given entity.
    pub fn contains(&self, entity: Entity) -> bool { self.get(entity).is_some() }

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
            let block = entity.0 / BLOCK_SIZE_U64;
            if current_block != block {
                block_vec = Some(
                    self.blocks
                        .entry(block)
                        .or_insert_with(|| Box::new([None; BLOCK_SIZE])),
                );
                current_block = block;
            }

            if let Some(ref mut vec) = block_vec {
                let idx = (entity.0 - block * BLOCK_SIZE_U64) as usize;
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
        let block = entity.0 / BLOCK_SIZE_U64;
        let idx = (entity.0 - block * BLOCK_SIZE_U64) as usize;
        if let Some(&result) = self.blocks.get(&block).and_then(|v| v.get(idx)) {
            result
        } else {
            None
        }
    }

    /// Removes an entity from the location map.
    pub fn remove(&mut self, entity: Entity) -> Option<EntityLocation> {
        let block = entity.0 / BLOCK_SIZE_U64;
        let idx = (entity.0 - block * BLOCK_SIZE_U64) as usize;
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

/// Allocates new entity IDs.
#[derive(Debug, Clone)]
pub struct EntityAllocator {
    next: Arc<AtomicU64>,
    stride: u64,
    offset: u64,
}

impl EntityAllocator {
    pub(crate) fn new(offset: u64, stride: u64) -> Self {
        assert!(stride > 0);
        Self {
            next: Arc::new(AtomicU64::new(offset * BLOCK_SIZE_U64)),
            stride,
            offset,
        }
    }

    /// Returns an iterator which yields new entity IDs.
    pub fn iter(&self) -> Allocate { Allocate::new(&self) }

    /// Returns `true` if the given entity ID lies within the address space of this allocator.
    pub fn address_space_contains(&self, entity: Entity) -> bool {
        let block = entity.0 / BLOCK_SIZE_U64;
        ((block - self.offset) % self.stride) == 0
    }

    fn next_block(&self) -> u64 {
        self.next
            .fetch_add(self.stride * BLOCK_SIZE_U64, Ordering::Relaxed)
    }

    /// Gets the block stride of the allocator.
    pub fn stride(&self) -> u64 { self.stride }

    /// Gets the block offset of the allocator.
    pub fn offset(&self) -> u64 { self.offset }

    pub(crate) fn head(&self) -> u64 { self.next.load(Ordering::Relaxed) }

    pub(crate) fn skip(&self, id: u64) {
        let mut block = id / BLOCK_SIZE_U64;

        // round up if we are part way through a block
        if id % BLOCK_SIZE_U64 != 0 {
            block += 1;
        }

        loop {
            let head = self.head();
            let current_block = head / BLOCK_SIZE_U64;
            if current_block >= block {
                break;
            }

            let new_block = block + (block - self.offset) % self.stride;

            if self
                .next
                .compare_and_swap(head, new_block, Ordering::Relaxed)
                == head
            {
                break;
            }
        }
    }
}

impl Default for EntityAllocator {
    fn default() -> Self { Self::new(0, 1) }
}

/// An iterator which yields new entity IDs.
pub struct Allocate<'a> {
    allocator: &'a EntityAllocator,
    base: u64,
    count: u64,
}

impl<'a> Allocate<'a> {
    fn new(allocator: &'a EntityAllocator) -> Self {
        Self {
            allocator,
            base: 0,
            count: 0,
        }
    }

    /// Returns `true` if the given ID has already been allocated.
    pub fn is_allocated(&mut self, Entity(entity): Entity) -> bool {
        let ceiling = self.base + BLOCK_SIZE_U64;
        let in_range = entity < self.base || (entity < ceiling && entity >= self.base + self.count);
        in_range && self.allocator.address_space_contains(Entity(entity))
    }
}

impl<'a> Iterator for Allocate<'a> {
    type Item = Entity;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            self.base = self.allocator.next_block();
            self.count = BLOCK_SIZE_U64;
        }

        self.count -= 1;
        Some(Entity(self.base + self.count))
    }
}
