use crate::entity::Entity;
use crate::event::{Event, Subscriber, Subscribers};
use crate::storage::components::{ComponentStorage, ComponentStorageWriter};
use crate::storage::components::{ComponentStorageLayout, ComponentTypeId, ComponentVec};
use crate::storage::index::ComponentIndex;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CHUNK_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkId(usize);

impl ChunkId {
    pub fn next_id() -> Self {
        Self(
            NEXT_CHUNK_ID
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .unwrap(),
        )
    }
}

/// A fixed capacity vector of entities which all contain the same
/// component types and tag values.
pub struct Chunk {
    id: ChunkId,
    entities: Vec<Entity>,
    components: ComponentStorage,
    subscribers: Subscribers,
}

impl Chunk {
    pub(crate) fn new(
        id: ChunkId,
        layout: &ComponentStorageLayout,
        mut subscribers: Subscribers,
    ) -> Self {
        subscribers.send(Event::ChunkCreated(id));

        Self {
            id,
            entities: Vec::with_capacity(layout.capacity()),
            components: layout.alloc_storage(),
            subscribers,
        }
    }

    pub fn capacity(&self) -> usize { self.entities.capacity() }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn is_full(&self) -> bool { self.len() == self.capacity() }

    pub fn components(&self) -> &ComponentStorage { &self.components }

    pub(crate) fn subscribe(&mut self, subscriber: Subscriber, send_initial_events: bool) {
        if send_initial_events {
            let _ = subscriber.try_send(Event::ChunkCreated(self.id));
            for entity in &self.entities {
                let _ = subscriber.try_send(Event::EntityInserted(*entity, self.id));
            }
        }

        self.subscribers.push(subscriber);
    }

    /// Removes the entity at the specified index by swapping it with the last entity.
    /// Returns the entity which was swapped into the vacated index.
    pub(crate) fn swap_remove(
        &mut self,
        index: ComponentIndex,
        drop_components: bool,
    ) -> Option<Entity> {
        self.components.swap_remove(index, drop_components);
        let removed = self.entities.swap_remove(index.0);

        self.subscribers
            .send(Event::EntityRemoved(removed, self.id));

        self.entities.get(index.0).copied()
    }

    pub(crate) fn begin_insert(&mut self) -> ChunkWriter {
        ChunkWriter {
            id: self.id,
            initial_count: self.entities.len(),
            entities: &mut self.entities,
            components: self.components.writer(),
            subscribers: &mut self.subscribers,
        }
    }
}

impl Deref for Chunk {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.entities }
}

pub struct ChunkWriter<'a> {
    id: ChunkId,
    initial_count: usize,
    entities: &'a mut Vec<Entity>,
    components: ComponentStorageWriter<'a>,
    subscribers: &'a mut Subscribers,
}

impl<'a> ChunkWriter<'a> {
    pub fn space(&self) -> usize { self.entities.capacity() - self.entities.len() }

    pub fn push(&mut self, entity: Entity) { self.entities.push(entity); }

    pub fn claim_components(&mut self, type_id: ComponentTypeId) -> Option<&'a mut ComponentVec> {
        self.components.claim(type_id)
    }

    pub fn inserted<'b>(&'b self) -> impl Iterator<Item = (ComponentIndex, Entity)> + 'b {
        let start = self.initial_count;
        self.entities[start..]
            .iter()
            .enumerate()
            .map(move |(i, e)| (ComponentIndex(i + start), *e))
    }
}

impl<'a> Drop for ChunkWriter<'a> {
    fn drop(&mut self) {
        for &entity in self.entities.iter().skip(self.initial_count) {
            self.subscribers
                .send(Event::EntityInserted(entity, self.id));
        }
    }
}

pub(crate) fn move_entity(
    source: &mut Chunk,
    index: ComponentIndex,
    target: &mut Chunk,
) -> Option<Entity> {
    debug_assert!(*index < source.len());
    debug_assert!(!target.is_full());

    let entity = source.entities[index.0];

    let source_components = &mut source.components;
    let mut target_writer = target.begin_insert();
    target_writer.push(entity);

    for (comp_type, source_slice) in source_components.writer().drain() {
        let element_size = source_slice.element_size();

        if let Some(target_slice) = target_writer.claim_components(comp_type) {
            // move the component into the target chunk
            unsafe {
                let source_ptr = source_slice.ptr_mut();
                let component = source_ptr.add(element_size * *index);
                target_slice.push_raw(NonNull::new_unchecked(component), 1);
            }
        } else {
            // drop the component rather than move it
            unsafe { source_slice.drop_in_place(index) };
        }
    }

    // remove the entity from this chunk
    source.swap_remove(index, false)
}
