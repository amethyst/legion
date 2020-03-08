use crate::entity::Entity;
use crate::storage::archetype::{ArchetypeId, ArchetypeTagsRef};
use crate::storage::chunk::ChunkId;
use crate::storage::components::ComponentTypeId;
use crate::storage::filter::{EntityFilterTuple, FilterResult, Passthrough};
use crate::storage::tags::TagTypeId;
use crate::storage::{ArchetypeFilter, LayoutFilter};
use crossbeam_channel::{Sender, TrySendError};
use std::iter::Iterator;
use std::sync::Arc;

/// Events emitted by a world to subscribers. See `World.subscribe(Sender, EntityFilter)`.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new archetype has been created.
    ArchetypeCreated(ArchetypeId),
    /// A new chunk has been created.
    ChunkCreated(ChunkId),
    /// An entity has been inserted into a chunk.
    EntityInserted(Entity, ChunkId),
    /// An entity has been removed from a chunk.
    EntityRemoved(Entity, ChunkId),
}

pub trait EventFilter: LayoutFilter + ArchetypeFilter {}

impl<L, A> EventFilter for EntityFilterTuple<L, A, Passthrough>
where
    L: LayoutFilter + Send + Sync + Clone,
    A: ArchetypeFilter + Send + Sync + Clone,
{
}

#[derive(Clone)]
pub struct Subscriber {
    pub filter: Arc<dyn EventFilter>,
    pub sender: Sender<Event>,
}

impl Subscriber {
    pub fn new(filter: Arc<dyn EventFilter>, sender: Sender<Event>) -> Self {
        Self { filter, sender }
    }

    pub fn filter(&self) -> &dyn EventFilter { &*self.filter }

    pub(crate) fn try_send(&self, message: Event) -> Result<(), TrySendError<Event>> {
        self.sender.try_send(message)
    }
}

#[derive(Clone)]
pub struct Subscribers {
    subscribers: Vec<Subscriber>,
}

impl Subscribers {
    pub fn new(subscribers: Vec<Subscriber>) -> Self { Self { subscribers } }

    pub fn push(&mut self, subscriber: Subscriber) { self.subscribers.push(subscriber); }

    pub fn send(&mut self, message: Event) {
        for i in (0..self.subscribers.len()).rev() {
            if let Err(error) = self.subscribers[i].try_send(message.clone()) {
                if let TrySendError::Disconnected(_) = error {
                    self.subscribers.swap_remove(i);
                }
            }
        }
    }

    pub fn matches_layout(
        &self,
        components: &[ComponentTypeId],
        tags: &[TagTypeId],
    ) -> Vec<Subscriber> {
        self.subscribers
            .iter()
            .filter(|sub| sub.filter.matches_layout(components, tags).is_pass())
            .cloned()
            .collect()
    }

    pub fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Vec<Subscriber> {
        self.subscribers
            .iter()
            .filter(|sub| sub.filter.matches_archetype(tags).is_pass())
            .cloned()
            .collect()
    }
}

impl Default for Subscribers {
    fn default() -> Self { Subscribers::new(Vec::new()) }
}
