use crate::borrow::{AtomicRefCell, Ref};
use crate::entity::Entity;
use crate::event::{Event, Subscriber, Subscribers};
use crate::storage::chunk::{move_entity, Chunk, ChunkId};
use crate::storage::components::{Component, ComponentMeta, ComponentTypeId};
use crate::storage::index::{ChunkIndex, ComponentIndex};
use crate::storage::tags::{Tag, TagMeta, TagTypeId};
use crate::storage::{ArchetypeFilter, EntityLayoutData, LayoutFilter, TagValues};
use std::fmt::{Debug, Formatter};
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::trace;

/// Number of empty chunks that are allowed to remain in an archetype before they are dropped.
const MAX_EMPTY_CHUNKS: usize = 1;
static NEXT_ARCHETYPE_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArchetypeId(usize);

impl ArchetypeId {
    pub fn next_id() -> Self {
        Self(
            NEXT_ARCHETYPE_ID
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .unwrap(),
        )
    }
}

/// Describes the layout of an entity, including what types of components
/// and tags shall be attached to the entity.
#[derive(Default, Clone, PartialEq)]
pub struct EntityTypeLayout {
    tags: Vec<TagTypeId>,
    tags_meta: Vec<TagMeta>,
    components: Vec<ComponentTypeId>,
    components_meta: Vec<ComponentMeta>,
    tag_names: Vec<&'static str>,
    component_names: Vec<&'static str>,
}

impl EntityTypeLayout {
    /// Gets a slice of the tags in the layout.
    pub fn tags(&self) -> impl Iterator<Item = (&TagTypeId, &TagMeta)> {
        self.tags.iter().zip(self.tags_meta.iter())
    }

    /// Gets a slice of the components in the layout.
    pub fn components(&self) -> impl Iterator<Item = (&ComponentTypeId, &ComponentMeta)> {
        self.components.iter().zip(self.components_meta.iter())
    }

    pub fn tag_types(&self) -> &[TagTypeId] { &self.tags }

    pub fn component_types(&self) -> &[ComponentTypeId] { &self.components }

    /// Adds a tag to the layout.
    pub fn register_tag_raw(&mut self, type_id: TagTypeId, type_meta: TagMeta) {
        self.assert_unique_tags(type_id);
        self.tags.push(type_id);
        self.tags_meta.push(type_meta);
        self.tag_names.push("<unknown>");
    }

    /// Adds a tag to the layout.
    pub fn register_tag<T: Tag>(&mut self) {
        let type_id = TagTypeId::of::<T>();
        self.assert_unique_tags(type_id);
        self.tags.push(type_id);
        self.tags_meta.push(TagMeta::of::<T>());
        self.tag_names.push(std::any::type_name::<T>());
    }

    /// Adds a component to the layout.
    pub fn register_component_raw(&mut self, type_id: ComponentTypeId, type_meta: ComponentMeta) {
        self.assert_unique_components(type_id);
        self.components.push(type_id);
        self.components_meta.push(type_meta);
        self.component_names.push("<unknown>");
    }

    /// Adds a component to the layout.
    pub fn register_component<T: Component>(&mut self) {
        let type_id = ComponentTypeId::of::<T>();
        self.assert_unique_components(type_id);
        self.components.push(type_id);
        self.components_meta.push(ComponentMeta::of::<T>());
        self.component_names.push(std::any::type_name::<T>());
    }

    pub fn has_component_id(&self, type_id: ComponentTypeId) -> bool {
        self.components.iter().any(|t| type_id == *t)
    }

    pub fn has_component<T: Component>(&self) -> bool {
        self.has_component_id(ComponentTypeId::of::<T>())
    }

    pub fn has_tag_id(&self, type_id: TagTypeId) -> bool { self.tags.iter().any(|t| type_id == *t) }

    pub fn has_tag<T: Tag>(&self) -> bool { self.has_tag_id(TagTypeId::of::<T>()) }

    fn assert_unique_components(&self, type_id: ComponentTypeId) {
        debug_assert!(
            self.components.iter().find(|t| type_id == **t).is_none(),
            "only one component of a given type may be attached to a single entity"
        );
    }

    fn assert_unique_tags(&self, type_id: TagTypeId) {
        debug_assert!(
            self.tags.iter().find(|t| type_id == **t).is_none(),
            "only one tag of a given type may be attached to a single entity"
        );
    }
}

impl LayoutFilter for EntityTypeLayout {
    fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
        Some(
            self.tags.len() == tags.len()
                && self.components.len() == components.len()
                && self.tags.iter().all(|t| tags.contains(t))
                && self.components.iter().all(|t| components.contains(t)),
        )
    }
}

impl Debug for EntityTypeLayout {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "EntityTypeLayout {{ tags: {:?}, components: {:?} }}",
            self.tag_names, self.component_names,
        )
    }
}

/// A reference to tag storage for an archetype.
pub struct ArchetypeTagsRef<'a> {
    tags: Ref<'a, TagValues>,
    index: usize,
}

impl<'a> ArchetypeTagsRef<'a> {
    pub(crate) fn new(tags: Ref<'a, TagValues>, index: usize) -> Self { Self { tags, index } }

    /// Gets a tag associated with the archetype.
    pub fn get<T: Tag>(&self) -> Option<&'a T> {
        let type_id = TagTypeId::of::<T>();
        let tags = self.tags.clone().into_ref();
        if let Some((_, storage)) = tags.tags.iter().find(|(t, _)| *t == type_id) {
            let slice = unsafe { storage.data_slice() };
            Some(&slice[self.index])
        } else {
            None
        }
    }

    pub fn get_raw(&self, type_id: TagTypeId) -> Option<(*const u8, &TagMeta)> {
        if let Some((_, storage)) = self.tags.tags.iter().find(|(t, _)| *t == type_id) {
            let (ptr, size, _) = storage.data_raw();
            Some((unsafe { ptr.add(size * self.index) }, storage.element()))
        } else {
            None
        }
    }
}

/// Contains all entities with a matching type layout and tag values.
pub struct Archetype {
    id: ArchetypeId,
    chunks: Vec<Chunk>,
    tags: Arc<AtomicRefCell<TagValues>>,
    tags_index: usize,
    layout: Arc<EntityLayoutData>,
    subscribers: Subscribers,
}

impl Archetype {
    pub(crate) fn new(
        tags: Arc<AtomicRefCell<TagValues>>,
        tags_index: usize,
        layout: Arc<EntityLayoutData>,
        mut subscribers: Subscribers,
    ) -> Self {
        let id = ArchetypeId::next_id();
        subscribers.send(Event::ArchetypeCreated(id));
        Self {
            id,
            chunks: Vec::new(),
            tags,
            tags_index,
            layout,
            subscribers,
        }
    }

    pub fn layout(&self) -> &EntityTypeLayout { &self.layout.entity_layout }

    pub fn tags(&self) -> ArchetypeTagsRef {
        ArchetypeTagsRef {
            tags: self.tags.get(),
            index: self.tags_index,
        }
    }

    pub fn subscribe(&mut self, subscriber: Subscriber, send_initial_events: bool) {
        if send_initial_events {
            let _ = subscriber.try_send(Event::ArchetypeCreated(self.id));
        }

        self.subscribers.push(subscriber.clone());

        for chunk in &mut self.chunks {
            chunk.subscribe(subscriber.clone(), send_initial_events);
        }
    }

    pub fn writable_chunk(&mut self) -> (ChunkIndex, &mut Chunk) {
        let index = if let Some((i, _)) = self
            .chunks
            .iter_mut()
            .enumerate()
            .find(|(_, chunk)| !chunk.is_full())
        {
            i
        } else {
            let chunk = Chunk::new(
                ChunkId::next_id(),
                &self.layout.chunk_layout,
                self.subscribers.clone(),
            );
            self.chunks.push(chunk);
            self.chunks.len() - 1
        };

        (ChunkIndex(index), &mut self.chunks[index])
    }

    pub(crate) fn defrag(
        &mut self,
        budget: &mut usize,
        on_moved: impl FnMut(Entity, ChunkIndex, ComponentIndex),
    ) -> bool {
        if self.chunks.is_empty() {
            return true;
        }

        let slice = &mut self.chunks;

        trace!("Defragmenting archetype");

        fn compact_chunks(
            slice: &mut [Chunk],
            budget: &mut usize,
            mut on_moved: impl FnMut(Entity, ChunkIndex, ComponentIndex),
        ) -> (bool, usize) {
            let mut first = 0;
            let mut last = slice.len() - 1;

            loop {
                // find the first chunk that is not full
                while first < last && slice[first].is_full() {
                    first += 1;
                }

                // find the last chunk that is not empty
                while last > first && slice[last].is_empty() {
                    last -= 1;
                }

                // exit if the cursors meet; the archetype is defragmented
                if first == last {
                    return (true, last);
                }

                // get mut references to both chunks
                let (with_first, with_last) = slice.split_at_mut(last);
                let target = &mut with_first[first];
                let source = &mut with_last[0];

                // move as many entities as we can from the last chunk into the first
                loop {
                    if *budget == 0 {
                        return (false, last);
                    }

                    *budget -= 1;

                    // move the last entity
                    let comp_index = ComponentIndex(source.len() - 1);
                    let swapped = move_entity(source, comp_index, target);
                    assert!(swapped.is_none());

                    // notify move
                    on_moved(*target.last().unwrap(), ChunkIndex(first), comp_index);

                    // exit if we cant move any more
                    if target.is_full() || source.is_empty() {
                        break;
                    }
                }
            }
        }

        // compact entities into lower chunks
        let (complete, last_occupied) = compact_chunks(slice, budget, on_moved);

        // remove excess chunks
        let keep = last_occupied + 1 + MAX_EMPTY_CHUNKS;
        while self.chunks.len() > keep {
            self.chunks.pop();
        }

        complete
    }
}

impl Index<ChunkIndex> for Archetype {
    type Output = Chunk;

    fn index(&self, ChunkIndex(index): ChunkIndex) -> &Self::Output { &self.chunks[index] }
}

impl IndexMut<ChunkIndex> for Archetype {
    fn index_mut(&mut self, ChunkIndex(index): ChunkIndex) -> &mut Self::Output {
        &mut self.chunks[index]
    }
}

impl Deref for Archetype {
    type Target = [Chunk];

    fn deref(&self) -> &Self::Target { &self.chunks }
}

impl DerefMut for Archetype {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.chunks }
}

impl ArchetypeFilter for Archetype {
    fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
        for (tag_id, meta) in self.layout.entity_layout.tags() {
            let (_, storage) = self
                .tags
                .get()
                .inner_ref()
                .tags
                .iter()
                .find(|(t, _)| t == tag_id)?;
            let (self_tag_ptr, size, _) = storage.data_raw();
            let (other_tag_ptr, _) = tags.get_raw(*tag_id)?;

            if unsafe { !meta.equals(self_tag_ptr.add(self.tags_index * size), other_tag_ptr) } {
                return Some(false);
            }
        }

        Some(true)
    }
}
