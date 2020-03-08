use crate::entity::{BlockAllocator, EntityAllocator};
use crate::world::World;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static NEXT_UNIVERSE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Default, Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct UniverseId(usize);

impl UniverseId {
    pub fn next_id() -> Self {
        Self(
            NEXT_UNIVERSE_ID
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .unwrap(),
        )
    }
}

/// The `Universe` is a factory for creating `World`s.
///
/// Entities inserted into worlds created within the same universe are guaranteed to have
/// unique `Entity` IDs, even across worlds.
#[derive(Debug)]
pub struct Universe {
    id: UniverseId,
    allocator: Arc<Mutex<BlockAllocator>>,
}

impl Universe {
    /// Creates a new `Universe`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            id: UniverseId::next_id(),
            allocator: Arc::new(Mutex::new(BlockAllocator::new())),
        }
    }

    /// Creates a new `World` within this `Universe`.
    ///
    /// Entities inserted into worlds created within the same universe are guaranteed to have
    /// unique `Entity` IDs, even across worlds. See also `World::new`.
    pub fn create_world(&self) -> World {
        World::new_with_allocator(EntityAllocator::new(self.allocator.clone()))
    }
}
