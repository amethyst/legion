use crate::borrow::{AtomicRefCell, Ref};
use crate::event::{Event, EventFilter, Subscriber, Subscribers};
use crate::storage::archetype::{Archetype, ArchetypeTagsRef, EntityTypeLayout};
use crate::storage::chunk::Chunk;
use crate::storage::components::{ComponentStorageLayout, ComponentTypeId};
use crate::storage::filter::FilterResult;
use crate::storage::index::ArchetypeIndex;
use crate::storage::slicevec::{SliceVec, SliceVecIter};
use crate::storage::tags::{TagStorage, TagTypeId};
use smallvec::SmallVec;
use std::iter::{Enumerate, Zip};
use std::ops::{Deref, DerefMut};
use std::slice::Iter;
use std::sync::Arc;

pub mod archetype;
pub mod chunk;
pub mod components;
pub mod filter;
pub mod index;
pub mod slicevec;
pub mod tags;

#[derive(Default)]
pub struct Storage {
    /// Query search index.
    index: LayoutIndex,
    /// Distinct entity component/tag layouts.
    /// Each index in this vector corresponds to an index in the `Index`.
    layouts: Vec<Arc<EntityLayoutData>>,
    /// A vector of all archetypes, each containing chunks of entity component data.
    data: Vec<Archetype>,
    /// All subscribers.
    subscribers: Subscribers,
    /// Subscribers for each layout.
    subscribers_per_layout: Vec<Subscribers>,
}

impl Storage {
    pub fn layout_index(&self) -> &LayoutIndex { &self.index }

    pub(crate) fn push_layout(&mut self, layout: EntityTypeLayout) -> usize {
        let storage_layout = ComponentStorageLayout::new(&layout);
        self.index.push_layout(&layout);
        self.subscribers_per_layout.push(Subscribers::new(
            self.subscribers
                .matches_layout(layout.component_types(), layout.tag_types()),
        ));
        self.layouts.push(Arc::new(EntityLayoutData {
            entity_layout: layout,
            chunk_layout: storage_layout,
        }));
        self.layouts.len() - 1
    }

    pub(crate) fn push_archetype(
        &mut self,
        layout_index: usize,
        mut init_tags: impl FnMut(&mut TagValues),
    ) -> ArchetypeIndex {
        {
            let mut tag_values = unsafe { self.index.tag_values[layout_index].get_mut() };
            init_tags(&mut *tag_values);
        }

        let index = ArchetypeIndex(self.data.len());
        let layout_archetypes = &mut self.index.archetypes[layout_index];
        let layout_archetypes_index = layout_archetypes.len();
        layout_archetypes.push(index);

        let tags = self.index.tag_values[layout_index].clone();
        let subscribers = self.subscribers_per_layout[layout_index]
            .matches_archetype(&ArchetypeTagsRef::new(tags.get(), layout_archetypes_index));
        let arch = Archetype::new(
            tags,
            layout_archetypes_index,
            self.layouts[layout_index].clone(),
            Subscribers::new(subscribers),
        );

        self.data.push(arch);
        index
    }

    pub(crate) fn merge(&mut self, mut other: Storage) {
        //        for archetype in other.data.drain(..) {
        //            let layout_index =
        //                {
        //                    if let Some((i, _)) = self.layout_index().iter().enumerate().find(
        //                        |(i, (tags, components, _, _))| {
        //                            archetype
        //                                .layout()
        //                                .matches_layout(tags, components)
        //                                .is_pass()
        //                        },
        //                    ) {
        //                        i
        //                    } else {
        //                        self.push_layout(archetype.layout().clone())
        //                    }
        //                };
        //
        //            let (tags, archetypes) =
        //                self.layouts.layout_archetypes(layout_index).unwrap();
        //
        //            for (index, arch) in archetypes.iter().enumerate() {
        //                if archetype.matches_archetype(&ArchetypeTagsRef { tags, index }).is_pass() {
        //                    // found matching archetype, move chunks
        //                    self.data[arch.0].
        //                }
        //            }
        //
        //            // no matching archetype
        //            self.data.push()
        //        }
        panic!("unimplemented");
    }

    pub(crate) fn subscribe<T: EventFilter + 'static>(
        &mut self,
        sender: crossbeam_channel::Sender<Event>,
        filter: T,
        send_initial_events: bool,
    ) {
        let subscriber = Subscriber::new(Arc::new(filter), sender);
        self.subscribers.push(subscriber.clone());

        for (i, (components, tags, _, _)) in self.index.iter().enumerate() {
            if subscriber
                .filter()
                .matches_layout(components, tags)
                .is_pass()
            {
                self.subscribers_per_layout[i].push(subscriber.clone());

                for arch in self.data.iter_mut() {
                    if subscriber
                        .filter()
                        .matches_archetype(&arch.tags())
                        .is_pass()
                    {
                        arch.subscribe(subscriber.clone(), send_initial_events);
                    }
                }
            }
        }
    }
}

impl Deref for Storage {
    type Target = [Archetype];

    fn deref(&self) -> &Self::Target { &self.data }
}

impl DerefMut for Storage {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.data }
}

#[derive(Default)]
pub struct LayoutIndex {
    /// Slices of component types for each entity layout.
    component_layouts: SliceVec<ComponentTypeId>,
    /// Slices of tag types for each entity layout.
    tag_layouts: SliceVec<TagTypeId>,
    /// Vectors of tag values for each entity layout.
    tag_values: Vec<Arc<AtomicRefCell<TagValues>>>,
    /// Vectors of archetype indexes for each entity layout.
    archetypes: Vec<Vec<ArchetypeIndex>>,
}

impl LayoutIndex {
    fn push_layout(&mut self, layout: &EntityTypeLayout) -> usize {
        self.component_layouts
            .push(layout.components().map(|(t, _)| *t));
        self.tag_layouts.push(layout.tags().map(|(t, _)| *t));
        self.tag_values.push(Arc::new(AtomicRefCell::new(TagValues {
            tags: layout
                .tags()
                .map(|(tag_type, meta)| (*tag_type, TagStorage::new(*meta)))
                .collect(),
        })));
        self.archetypes.push(Vec::default());
        self.archetypes.len() - 1
    }

    pub fn iter(&self) -> LayoutIndexIter {
        let layouts = self
            .component_layouts
            .iter()
            .zip(self.tag_layouts.iter())
            .zip(self.tag_values.iter())
            .zip(self.archetypes.iter());
        LayoutIndexIter { layouts }
    }

    pub fn layout_archetypes(
        &self,
        layout_index: usize,
    ) -> Option<(Ref<TagValues>, &[ArchetypeIndex])> {
        let tags = self.tag_values.get(layout_index)?;
        let archetypes = self.archetypes.get(layout_index)?;
        Some((tags.get(), archetypes.as_slice()))
    }

    pub fn search<'a, T: LayoutFilter, U: ArchetypeFilter>(
        &'a self,
        layout_filter: &'a T,
        archetype_filter: &'a U,
    ) -> ArchetypeSearchIter<'a, T, U> {
        ArchetypeSearchIter {
            layout_filter,
            archetype_filter,
            layout_data: self.iter(),
            current_layout: None,
        }
    }
}

pub struct LayoutIndexIter<'a> {
    layouts: Zip<
        Zip<
            Zip<SliceVecIter<'a, ComponentTypeId>, SliceVecIter<'a, TagTypeId>>,
            Iter<'a, Arc<AtomicRefCell<TagValues>>>,
        >,
        Iter<'a, Vec<ArchetypeIndex>>,
    >,
}

impl<'a> Iterator for LayoutIndexIter<'a> {
    type Item = (
        &'a [ComponentTypeId],
        &'a [TagTypeId],
        Ref<'a, TagValues>,
        &'a [ArchetypeIndex],
    );

    fn next(&mut self) -> Option<Self::Item> {
        self.layouts
            .next()
            .map(|(((component_types, tag_types), tags), archetypes)| {
                (
                    component_types,
                    tag_types,
                    tags.get(),
                    archetypes.as_slice(),
                )
            })
    }
}

#[derive(Default)]
pub struct TagValues {
    tags: SmallVec<[(TagTypeId, TagStorage); 3]>,
}

impl TagValues {
    pub fn get(&self, type_id: TagTypeId) -> Option<&TagStorage> {
        self.tags
            .iter()
            .find(|(t, _)| t == &type_id)
            .map(|(_, t)| t)
    }

    pub fn get_mut(&mut self, type_id: TagTypeId) -> Option<&mut TagStorage> {
        self.tags
            .iter_mut()
            .find(|(t, _)| t == &type_id)
            .map(|(_, t)| t)
    }
}

#[derive(Clone)]
pub struct EntityLayoutData {
    pub entity_layout: EntityTypeLayout,
    pub chunk_layout: ComponentStorageLayout,
}

pub trait LayoutFilter {
    fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool>;
}

impl<T: LayoutFilter> LayoutFilter for &T {
    fn matches_layout(&self, components: &[ComponentTypeId], tags: &[TagTypeId]) -> Option<bool> {
        <T as LayoutFilter>::matches_layout(self, components, tags)
    }
}

pub trait ArchetypeFilter {
    fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool>;
}

impl<T: ArchetypeFilter> ArchetypeFilter for &T {
    fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
        <T as ArchetypeFilter>::matches_archetype(self, tags)
    }
}

pub trait ChunkFilter {
    fn prepare(&mut self);
    fn matches_chunk(&mut self, chunk: &Chunk) -> Option<bool>;
}

pub struct ArchetypeSearchIter<'a, Layout, Arch>
where
    Layout: LayoutFilter,
    Arch: ArchetypeFilter,
{
    layout_filter: &'a Layout,
    archetype_filter: &'a Arch,
    layout_data: LayoutIndexIter<'a>,
    current_layout: Option<(Ref<'a, TagValues>, Enumerate<Iter<'a, ArchetypeIndex>>)>,
}

impl<'a, Layout, Arch> Iterator for ArchetypeSearchIter<'a, Layout, Arch>
where
    Layout: LayoutFilter,
    Arch: ArchetypeFilter,
{
    type Item = ArchetypeIndex;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // if we have selected a layout, find next arch which matches arch filter
            if let Some((tags, archetypes)) = &mut self.current_layout {
                for (index, arch) in archetypes {
                    if self
                        .archetype_filter
                        .matches_archetype(&ArchetypeTagsRef::new(tags.clone(), index))
                        .is_pass()
                    {
                        return Some(*arch);
                    }
                }
            }

            // find next layout
            loop {
                match self.layout_data.next() {
                    Some((component_types, tag_types, tags, archetypes)) => {
                        if self
                            .layout_filter
                            .matches_layout(component_types, tag_types)
                            .is_pass()
                        {
                            self.current_layout = Some((tags, archetypes.iter().enumerate()));
                            break;
                        }
                    }
                    None => return None,
                }
            }
        }
    }
}
