use crate::filter::{
    ArchetypeFilterData, ChunkFilterData, ChunksetFilterData, EntityFilter, Filter, FilterResult,
};
use crate::storage::ArchetypeId;
use crate::storage::ChunkId;
use crate::{entity::Entity, world::WorldId};
use crossbeam::channel::{Sender, TrySendError};
use crossbeam::queue::{ArrayQueue, PushError};
use derivative::Derivative;
use rayon::prelude::*;
use shrinkwraprs::Shrinkwrap;
use std::marker::PhantomData;
use std::sync::Arc;

#[derive(Clone)]
pub enum Event {
    ArchetypeCreated(ArchetypeId),
    ChunkCreated(ChunkId),
    EntityCreated(Entity),
    EntityDeleted(Entity),
    EntityAdded(Entity, ChunkId),
    EntityRemoved(Entity, ChunkId),
}

pub(crate) trait EventFilter: Send + Sync + 'static {
    fn matches_archetype(&self, data: ArchetypeFilterData, index: usize) -> bool;
    fn matches_chunkset(&self, data: ChunksetFilterData, index: usize) -> bool;
    fn matches_chunk(&self, data: ChunkFilterData, index: usize) -> bool;
}

pub(crate) struct EventFilterWrapper<T: EntityFilter + Sync + 'static>(pub T);

impl<T: EntityFilter + Sync + 'static> EventFilter for EventFilterWrapper<T> {
    fn matches_archetype(&self, data: ArchetypeFilterData, index: usize) -> bool {
        let (filter, _, _) = self.0.filters();
        if let Some(element) = filter.collect(data).nth(index) {
            return filter.is_match(&element).is_pass();
        }

        false
    }

    fn matches_chunkset(&self, data: ChunksetFilterData, index: usize) -> bool {
        let (_, filter, _) = self.0.filters();
        if let Some(element) = filter.collect(data).nth(index) {
            return filter.is_match(&element).is_pass();
        }

        false
    }

    fn matches_chunk(&self, data: ChunkFilterData, index: usize) -> bool {
        let (_, _, filter) = self.0.filters();
        if let Some(element) = filter.collect(data).nth(index) {
            return filter.is_match(&element).is_pass();
        }

        false
    }
}

#[derive(Clone)]
pub(crate) struct Subscriber {
    pub filter: Arc<dyn EventFilter>,
    pub sender: Sender<Event>,
}

impl Subscriber {
    pub fn new(filter: Arc<dyn EventFilter>, sender: Sender<Event>) -> Self {
        Self { filter, sender }
    }
}

#[derive(Clone)]
pub(crate) struct Subscribers {
    subscribers: Vec<Subscriber>,
}

impl Subscribers {
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
        }
    }

    pub fn push(&mut self, subscriber: Subscriber) { self.subscribers.push(subscriber); }

    pub fn send(&mut self, message: Event) {
        for i in (0..self.subscribers.len()).rev() {
            if let Err(error) = self.subscribers[i].sender.try_send(message.clone()) {
                if let TrySendError::Disconnected(_) = error {
                    self.subscribers.swap_remove(i);
                }
            }
        }
    }

    pub fn matches_archetype(&self, data: ArchetypeFilterData, index: usize) -> Self {
        let subscribers = self
            .subscribers
            .iter()
            .filter(|sub| sub.filter.matches_archetype(data, index))
            .cloned()
            .collect();
        Self { subscribers }
    }

    pub fn matches_chunkset(&self, data: ChunksetFilterData, index: usize) -> Self {
        let subscribers = self
            .subscribers
            .iter()
            .filter(|sub| sub.filter.matches_chunkset(data, index))
            .cloned()
            .collect();
        Self { subscribers }
    }

    pub fn matches_chunk(&self, data: ChunkFilterData, index: usize) -> Self {
        let subscribers = self
            .subscribers
            .iter()
            .filter(|sub| sub.filter.matches_chunk(data, index))
            .cloned()
            .collect();
        Self { subscribers }
    }
}

impl Default for Subscribers {
    fn default() -> Self { Subscribers::new() }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ListenerId(usize);

/// This queue performs per-listener queueing using a crossbeam `ArrayQueue`, pre-defined to an
/// upper limit of messages allowed.
#[derive(Derivative)]
#[derivative(Debug(bound = "T: std::fmt::Debug"))]
pub struct Channel<T> {
    queues: Vec<ArrayQueue<T>>,
    #[derivative(Debug = "ignore")]
    bound_functions: Vec<Box<dyn Fn(T) -> Option<T> + Send + Sync>>,
}

impl<T: Clone> Channel<T> {
    pub fn bind_listener(&mut self, message_capacity: usize) -> ListenerId {
        let new_id = self.queues.len();
        self.queues.push(ArrayQueue::new(message_capacity));

        ListenerId(new_id)
    }

    pub fn bind_exec(&mut self, f: Box<dyn Fn(T) -> Option<T> + Send + Sync>) {
        self.bound_functions.push(f);
    }

    pub fn read(&self, listener_id: ListenerId) -> Option<T> {
        self.queues[listener_id.0].pop().ok()
    }

    pub fn write_iter(&self, iter: impl Iterator<Item = T>) -> Result<(), PushError<T>>
    where
        T: Sync + Send,
    {
        for event in iter {
            self.write(event)?;
        }

        Ok(())
    }

    pub fn write(&self, event: T) -> Result<(), PushError<T>>
    where
        T: Sync + Send,
    {
        if !self
            .bound_functions
            .par_iter()
            .map(|f| (f)(event.clone()))
            .any(|e| e.is_none())
        {
            self.queues
                .par_iter()
                .for_each(|queue| queue.push(event.clone()).unwrap());
        }

        Ok(())
    }
}

impl<T> Default for Channel<T> {
    fn default() -> Self {
        Self {
            queues: Vec::new(),
            bound_functions: Vec::new(),
        }
    }
}

#[derive(Shrinkwrap, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldCreatedEvent(pub WorldId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentEvent {
    ComponentAdded,
    ComponentRemoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityEvent {
    Created(Entity),
    Deleted(Entity),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityFilterEvent<F: EntityFilter> {
    InScope(Entity, PhantomData<F>),
    OutScope(Entity, PhantomData<F>),
}
