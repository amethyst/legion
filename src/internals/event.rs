use super::entity::Entity;
use super::query::filter::LayoutFilter;
use super::storage::{
    archetype::{Archetype, ArchetypeIndex},
    component::ComponentTypeId,
};
use std::iter::Iterator;
use std::{fmt::Debug, sync::Arc};

/// Events emitted by a world to subscribers. See `World.subscribe(Sender, EntityFilter)`.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new archetype has been created.
    ArchetypeCreated(ArchetypeIndex),
    /// An entity has been inserted into an archetype.
    EntityInserted(Entity, ArchetypeIndex),
    /// An entity has been removed from an archetype.
    EntityRemoved(Entity, ArchetypeIndex),
}

/// Describes a type which can send entity events.
pub trait EventSender: Send + Sync {
    /// Sends the given event to all listeners.
    /// Returns `true` if the sender is still alive.
    fn send(&self, event: Event) -> bool;
}

#[cfg(feature = "crossbeam-events")]
impl EventSender for crossbeam_channel::Sender<Event> {
    fn send(&self, event: Event) -> bool {
        !matches!(
            self.try_send(event),
            Err(crossbeam_channel::TrySendError::Disconnected(_))
        )
    }
}

#[derive(Clone)]
pub(crate) struct Subscriber {
    filter: Arc<dyn LayoutFilter>,
    sender: Arc<dyn EventSender>,
}

impl Subscriber {
    pub(crate) fn new<F: LayoutFilter + 'static, S: EventSender + 'static>(
        filter: F,
        sender: S,
    ) -> Self {
        Self {
            filter: Arc::new(filter),
            sender: Arc::new(sender),
        }
    }

    pub(crate) fn is_interested(&self, archetype: &Archetype) -> bool {
        self.filter
            .matches_layout(archetype.layout().component_types())
            .is_pass()
    }

    pub(crate) fn send(&self, message: Event) -> bool {
        self.sender.send(message)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Subscribers {
    subscribers: Vec<Subscriber>,
}

impl Subscribers {
    pub fn push(&mut self, subscriber: Subscriber) {
        self.subscribers.push(subscriber);
    }

    pub fn send(&mut self, message: Event) {
        for i in (0..self.subscribers.len()).rev() {
            if !self.subscribers[i].send(message.clone()) {
                self.subscribers.swap_remove(i);
            }
        }
    }

    pub fn matches_layout(&self, components: &[ComponentTypeId]) -> Self {
        Self {
            subscribers: self
                .subscribers
                .iter()
                .filter(|sub| sub.filter.matches_layout(components).is_pass())
                .cloned()
                .collect(),
        }
    }
}

impl Debug for Subscribers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscribers")
            .field("len", &self.subscribers.len())
            .finish()
    }
}
