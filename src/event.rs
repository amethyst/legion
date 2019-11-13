use crate::{entity::Entity, filter::EntityFilter, world::WorldId};
use derivative::Derivative;
use shrinkwraprs::Shrinkwrap;
use std::{cell::RefCell, collections::VecDeque, marker::PhantomData};

#[cfg(feature = "par-iter")]
use rayon::prelude::*;

#[cfg(feature = "par-iter")]
use crossbeam::queue::{ArrayQueue, PopError, PushError};
use crate::storage::ArchetypeId;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ListenerId(usize);

/// This queue performs per-listener queueing using a crossbeam `ArrayQueue`, pre-defined to an
/// upper limit of messages allowed.
#[derive(Derivative)]
#[derivative(Debug(bound = "T: std::fmt::Debug"))]
pub struct Channel<T> {
    #[cfg(not(feature = "par-iter"))]
    queues: Vec<RefCell<VecDeque<T>>>,

    #[cfg(feature = "par-iter")]
    queues: Vec<ArrayQueue<T>>,

    #[derivative(Debug = "ignore")]
    #[cfg(feature = "par-iter")]
    bound_functions: Vec<Box<dyn Fn(T) -> Option<T> + Send + Sync>>,

    #[derivative(Debug = "ignore")]
    #[cfg(not(feature = "par-iter"))]
    bound_functions: Vec<Box<dyn Fn(T) -> Option<T>>>,
}

impl<T: Copy> Channel<T> {
    pub fn bind_listener(&mut self, message_capacity: usize) -> ListenerId {
        let new_id = self.queues.len();
        #[cfg(feature = "par-iter")]
        {
            self.queues.push(ArrayQueue::new(message_capacity));
        }

        #[cfg(not(feature = "par-iter"))]
        {
            self.queues
                .push(RefCell::new(VecDeque::with_capacity(message_capacity)));
        }

        ListenerId(new_id)
    }

    pub fn bind_exec(&mut self, f: Box<dyn Fn(T) -> Option<T> + Send + Sync>) {
        self.bound_functions.push(f);
    }

    #[cfg(not(feature = "par-iter"))]
    pub fn read(&self, listener_id: ListenerId) -> Option<T> {
        self.queues[listener_id.0].borrow_mut().pop_front()
    }

    #[cfg(feature = "par-iter")]
    pub fn read(&self, listener_id: ListenerId) -> Option<T> {
        self.queues[listener_id.0].pop().ok()
    }

    #[cfg(not(feature = "par-iter"))]
    pub fn write_iter(&self, iter: impl Iterator<Item = T>) -> Result<(), ()>
    where
        T: Send,
    {
        for event in iter {
            self.write(event)?;
        }

        Ok(())
    }

    #[cfg(feature = "par-iter")]
    pub fn write_iter(&self, iter: impl Iterator<Item = T>) -> Result<(), PushError<T>>
    where
        T: Sync + Send,
    {
        for event in iter {
            self.write(event)?;
        }

        Ok(())
    }

    #[cfg(feature = "par-iter")]
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

    #[cfg(not(feature = "par-iter"))]
    pub fn write(&self, event: T) -> Result<(), ()> {
        if let Some(event) = self
            .bound_functions
            .iter()
            .try_fold(event, |_, f| (f)(event))
        {
            // Propigate the event to all the queues.
            for queue in &self.queues {
                queue.borrow_mut().push_front(event);
            }
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
pub enum ArchetypeEvent {
    /// Indicates that an archetype was created.
    Created(ArchetypeId),
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
