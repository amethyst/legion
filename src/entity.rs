use crate::{
    hash::U64Hasher,
    storage::{archetype::ArchetypeIndex, ComponentIndex},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct Entity(u64);

#[derive(Debug, Copy, Clone)]
pub struct EntityLocation(pub(crate) ArchetypeIndex, pub(crate) ComponentIndex);

impl EntityLocation {
    pub fn new(archetype: ArchetypeIndex, component: ComponentIndex) -> Self {
        EntityLocation(archetype, component)
    }

    pub fn archetype(&self) -> ArchetypeIndex { self.0 }

    pub fn component(&self) -> ComponentIndex { self.1 }
}

pub type EntityHasher = BuildHasherDefault<U64Hasher>;

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
    pub fn len(&self) -> usize { self.len }

    pub fn contains(&self, entity: Entity) -> bool { self.get(entity).is_some() }

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

    pub fn set(&mut self, entity: Entity, location: EntityLocation) {
        self.insert(&[entity], location.archetype(), location.component());
    }

    pub fn get(&self, entity: Entity) -> Option<EntityLocation> {
        let block = entity.0 / BLOCK_SIZE_U64;
        let idx = (entity.0 - block * BLOCK_SIZE_U64) as usize;
        if let Some(&result) = self.blocks.get(&block).and_then(|v| v.get(idx)) {
            result
        } else {
            None
        }
    }

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

#[derive(Debug, Clone)]
pub struct EntityAllocator {
    next: Arc<AtomicU64>,
    stride: u64,
    offset: u64,
}

impl EntityAllocator {
    pub fn new(offset: u64, stride: u64) -> Self {
        assert!(stride > 0);
        Self {
            next: Arc::new(AtomicU64::new(offset * BLOCK_SIZE_U64)),
            stride,
            offset,
        }
    }

    pub fn iter(&self) -> Allocate { Allocate::new(&self) }

    pub fn address_space_contains(&self, entity: Entity) -> bool {
        let block = entity.0 / BLOCK_SIZE_U64;
        ((block - self.offset) % self.stride) == 0
    }

    fn next_block(&self) -> u64 {
        self.next
            .fetch_add(self.stride * BLOCK_SIZE_U64, Ordering::Relaxed)
    }

    pub fn stride(&self) -> u64 { self.stride }

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
