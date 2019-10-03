use crate::entity::Entity;
use crossbeam::queue::{ArrayQueue, PopError, PushError};

#[cfg(feature = "par-iter")]
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ListenerId(usize);

/// This queue performs per-listener queueing using a crossbeam `ArrayQueue`, pre-defined to an
/// upper limit of messages allowed.
#[derive(Debug)]
pub struct Channel<T> {
    queues: Vec<ArrayQueue<T>>,
}

impl<T: Send + Copy> Channel<T> {
    pub fn bind_listener(&mut self, message_capacity: usize) -> ListenerId {
        let new_id = self.queues.len();
        self.queues.push(ArrayQueue::new(message_capacity));

        ListenerId(new_id)
    }

    pub fn read(&self, listener_id: ListenerId) -> Result<T, PopError> {
        self.queues[listener_id.0].pop()
    }

    pub fn write(&self, event: T) -> Result<(), PushError<T>> {
        /*#[cfg(feature = "par-iter")]
        {
            self.queues.par_iter().map(move |queue| queue.push(event));
            // TODO: we should try_fold/try_reduce these errors
            Ok(())
        }

        #[cfg(not(feature = "par-iter"))]
        */
        {
            for queue in &self.queues {
                queue.push(event)?;
            }

            Ok(())
        }
    }
}

impl<T> Default for Channel<T> {
    fn default() -> Self { Self { queues: Vec::new() } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldCreatedEvent {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentEvent {
    ComponentAdded,
    ComponentRemoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityEvent {
    EntityCreated(Entity),
    EntityRemoved(Entity),
}

#[cfg(test)]
mod tests {
    #[test]
    fn simple_events() {
        println!("Hello World");
    }
}
