use crate::storage::{archetype::ArchetypeIndex, ComponentIndex};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

const BLOCK_SIZE: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Entity(u64);

#[derive(Debug, Copy, Clone)]
pub struct EntityLocation(pub(crate) ArchetypeIndex, pub(crate) ComponentIndex);

impl EntityLocation {
    pub fn new(archetype: ArchetypeIndex, component: ComponentIndex) -> Self {
        EntityLocation(archetype, component)
    }

    pub fn archetype(&self) -> ArchetypeIndex {
        self.0
    }

    pub fn component(&self) -> ComponentIndex {
        self.1
    }
}

#[derive(Debug, Clone)]
pub struct EntityAllocator {
    next: Arc<AtomicU64>,
    stride: u64,
}

impl EntityAllocator {
    pub fn new(offset: u64, stride: u64) -> Self {
        assert!(stride > 0);
        Self {
            next: Arc::new(AtomicU64::new(offset * BLOCK_SIZE)),
            stride: stride * BLOCK_SIZE,
        }
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = Entity> + 'a {
        std::iter::repeat_with(move || {
            self.next
                .fetch_add(self.stride * BLOCK_SIZE, Ordering::Relaxed)
        })
        .flat_map(|base| base..(base + BLOCK_SIZE))
        .map(Entity)
    }
}

impl Default for EntityAllocator {
    fn default() -> Self {
        Self::new(0, 1)
    }
}
