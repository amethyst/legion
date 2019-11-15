use crate::{entity::Entity, filter::EntityFilter, world::WorldId};
use crossbeam::queue::{ArrayQueue, PushError};
use derivative::Derivative;
use rayon::prelude::*;
use shrinkwraprs::Shrinkwrap;
use std::marker::PhantomData;

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

impl<T: Copy> Channel<T> {
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
            .map(|f| (f)(event))
            .any(|e| e.is_none())
        {
            self.queues
                .par_iter()
                .for_each(|queue| queue.push(event).unwrap());
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
