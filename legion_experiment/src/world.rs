use crate::entity::{BlockAllocator, Entity, EntityAllocator, EntityLocation, Locations};
use crate::entry::{Entry, EntryMut};
use crate::event::{Event, EventFilter};
use crate::storage::archetype::{Archetype, ArchetypeTagsRef, EntityTypeLayout};
use crate::storage::chunk::ChunkWriter;
use crate::storage::components::{Component, ComponentTypeId};
use crate::storage::filter::{And, FilterResult};
use crate::storage::index::ArchetypeIndex;
use crate::storage::tags::{Tag, TagTypeId};
use crate::storage::{ArchetypeFilter, LayoutFilter, Storage, TagValues};
use parking_lot::Mutex;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{info, span, trace, Level};

static NEXT_WORLD_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorldId(usize);

impl WorldId {
    pub fn next_id() -> Self {
        Self(
            NEXT_WORLD_ID
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .unwrap(),
        )
    }
}

pub struct World {
    id: WorldId,
    len: usize,
    storage: Storage,
    entity_allocator: EntityAllocator,
    entity_locations: Locations,
    allocation_buffer: Vec<Entity>,
    defrag_progress: usize,
}

impl Default for World {
    fn default() -> Self {
        Self::new_with_allocator(EntityAllocator::new(Arc::new(Mutex::new(
            BlockAllocator::new(),
        ))))
    }
}

impl World {
    pub fn new() -> Self { Self::default() }

    pub(crate) fn new_with_allocator(allocator: EntityAllocator) -> Self {
        Self {
            id: WorldId::next_id(),
            len: 0,
            storage: Storage::default(),
            entity_allocator: allocator,
            entity_locations: Locations::new(),
            allocation_buffer: Vec::default(),
            defrag_progress: 0,
        }
    }

    pub fn subscribe<T: EventFilter + 'static>(
        &mut self,
        sender: crossbeam_channel::Sender<Event>,
        filter: T,
        send_initial_events: bool,
    ) {
        self.storage.subscribe(sender, filter, send_initial_events)
    }

    pub fn len(&self) -> usize { self.len }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn contains(&self, entity: Entity) -> bool {
        self.entity_allocator.is_alive(entity) && self.entity_locations.get(entity).is_some()
    }

    pub fn push<T>(&mut self, tags: impl TagSet, components: T) -> &[Entity]
    where
        Vec<T>: IntoComponentSource,
    {
        self.extend(tags, vec![components])
    }

    pub fn extend(
        &mut self,
        mut tags: impl TagSet,
        components: impl IntoComponentSource,
    ) -> &[Entity] {
        let mut components = components.into();

        let span = span!(Level::TRACE, "Inserting entities", world = self.id.0);
        let _guard = span.enter();

        self.allocation_buffer.clear();
        self.allocation_buffer.reserve(components.size_hint().0);

        // find an appropriate archetype
        let arch_index = self.get_archetype(&mut tags, &mut components);
        let arch = &mut self.storage[arch_index];

        // insert entities into chunks until there are none left to insert
        loop {
            // find a chunk and write as many entities as we can
            let (chunk_index, chunk) = arch.writable_chunk();
            let mut writer = chunk.begin_insert();
            let complete =
                components.push_components(&mut writer, self.entity_allocator.create_entities());

            // record the new entity locations
            for (i, e) in writer.inserted() {
                let location = EntityLocation::new(arch_index, chunk_index, i);
                self.entity_locations.set(e, location);
                self.allocation_buffer.push(e);
                self.len += 1;
            }

            if complete {
                break;
            }
        }

        trace!(count = self.allocation_buffer.len(), "Inserted entities");
        &self.allocation_buffer
    }

    pub fn remove(&mut self, entity: Entity) -> bool {
        if !self.entity_allocator.is_alive(entity) {
            return false;
        }

        if self.entity_allocator.delete_entity(entity) {
            let location = self.entity_locations.get(entity).unwrap();
            let archetype = &mut self.storage[location.archetype()];
            let chunk = &mut archetype[location.chunk()];

            if let Some(swapped) = chunk.swap_remove(location.component(), true) {
                self.entity_locations.set(swapped, location);
            }

            self.len -= 1;
            trace!(world = self.id.0, ?entity, "Deleted entity");

            true
        } else {
            false
        }
    }

    pub fn entity(&self, entity: Entity) -> Option<Entry> {
        if !self.entity_allocator.is_alive(entity) {
            None
        } else {
            self.entity_locations
                .get(entity)
                .map(|location| Entry::new(location, &self.storage))
        }
    }

    pub fn entity_mut(&mut self, entity: Entity) -> Option<EntryMut> {
        if !self.entity_allocator.is_alive(entity) {
            None
        } else {
            self.entity_locations.get(entity).map(move |location| {
                EntryMut::new(location, &mut self.storage, &mut self.entity_locations)
            })
        }
    }

    pub fn defrag(&mut self, budget: Option<usize>) {
        let span = span!(
            Level::INFO,
            "Defragmenting",
            world = self.id.0,
            start_archetype = self.defrag_progress
        );
        let _guard = span.enter();

        let mut budget = budget.unwrap_or(std::usize::MAX);
        let start = self.defrag_progress;
        while self.defrag_progress < self.storage.len() {
            let arch_index = ArchetypeIndex(self.defrag_progress);
            let entity_locations = &mut self.entity_locations;
            let complete =
                self.storage[arch_index].defrag(&mut budget, |entity, chunk, component| {
                    entity_locations.set(entity, EntityLocation::new(arch_index, chunk, component));
                });
            if complete {
                self.defrag_progress = (self.defrag_progress + 1) % self.storage.len();
            }

            if budget == 0 || self.defrag_progress == start {
                break;
            }
        }
    }

    pub fn merge(&mut self, mut other: World) {
        // todo implement merge
        panic!("unimplemented");
    }

    fn get_archetype<Tags: TagSet, Components: ComponentSource>(
        &mut self,
        tags: &mut Tags,
        components: &mut Components,
    ) -> ArchetypeIndex {
        let layout_index = {
            let maybe_found = self
                .storage
                .layout_index()
                .iter()
                .enumerate()
                .filter(|(_, (tag_types, component_types, _, _))| {
                    And {
                        filters: (&*tags, &*components),
                    }
                    .matches_layout(tag_types, component_types)
                    .is_pass()
                })
                .map(|(i, _)| i)
                .next();
            if let Some(index) = maybe_found {
                index
            } else {
                let mut layout = EntityTypeLayout::default();
                tags.tailor_layout(&mut layout);
                components.tailor_layout(&mut layout);
                self.storage.push_layout(layout)
            }
        };

        let maybe_found = {
            let (tag_values, archetypes) = self
                .storage
                .layout_index()
                .layout_archetypes(layout_index)
                .unwrap();
            archetypes.iter().enumerate().find(|(i, _)| {
                tags.matches_archetype(&ArchetypeTagsRef::new(tag_values.clone(), *i))
                    .is_pass()
            })
        };

        if let Some((_, archetype)) = maybe_found {
            *archetype
        } else {
            self.storage
                .push_archetype(layout_index, move |t| tags.write_archetype_tags(t))
        }
    }
}

pub trait TagSet: LayoutFilter + ArchetypeFilter {
    fn tailor_layout(&mut self, layout: &mut EntityTypeLayout);
    fn write_archetype_tags(&mut self, tags: &mut TagValues);
}

macro_rules! tag_tuple {
    () => (
        impl_tag_tuple!();
    );
    ($head_ty:ident) => {
        impl_tag_tuple!($head_ty);
        tag_tuple!();
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_tag_tuple!($head_ty, $( $tail_ty ),*);
        tag_tuple!($( $tail_ty ),*);
    );
}

macro_rules! impl_tag_tuple {
    () => {
        impl LayoutFilter for () {
            fn matches_layout(
                &self,
                _: &[ComponentTypeId],
                tags: &[TagTypeId],
            ) -> Option<bool> {
                Some(tags.len() == 0)
            }
        }

        impl ArchetypeFilter for () {
            fn matches_archetype(&self, _: &ArchetypeTagsRef) -> Option<bool> { Some(true) }
        }

        impl TagSet for () {
            fn tailor_layout(&mut self, _: &mut EntityTypeLayout) {}
            fn write_archetype_tags(&mut self, _: &mut TagValues) {}
        }
    };
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Tag ),*> LayoutFilter for ($( $ty, )*) {
            fn matches_layout(
                &self,
                _: &[ComponentTypeId],
                tags: &[TagTypeId],
            ) -> Option<bool> {
                let types = &[$( TagTypeId::of::<$ty>() ),*];
                Some(tags.len() == types.len() && types.iter().all(|t| tags.contains(t)))
            }
        }

        impl<$( $ty: Tag ),*> ArchetypeFilter for ($( $ty, )*) {
            fn matches_archetype(&self, tags: &ArchetypeTagsRef) -> Option<bool> {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self;
                Some($( Some($ty) == tags.get() )&&*)
            }
        }

        impl<$( $ty: Tag ),*> TagSet for ($( $ty, )*) {
            fn tailor_layout(&mut self, layout: &mut EntityTypeLayout) {
                $(
                    layout.register_tag::<$ty>();
                )*
            }

            paste::item! {
                fn write_archetype_tags(&mut self, tags: &mut TagValues) {
                    #![allow(unused_variables)]
                    #![allow(non_snake_case)]
                    let ($( [<$ty _value>], )*) = self;
                    $(
                        let storage = tags.get_mut(TagTypeId::of::<$ty>()).unwrap();
                        unsafe { storage.push([<$ty _value>].clone()) };
                    )*
                }
            }
        }
    };
}

tag_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

pub trait ComponentSource: LayoutFilter {
    fn tailor_layout(&mut self, layout: &mut EntityTypeLayout);
    fn push_components(
        &mut self,
        chunk: &mut ChunkWriter,
        entities: impl Iterator<Item = Entity>,
    ) -> bool;
    fn size_hint(&self) -> (usize, Option<usize>) { (0, None) }
}

pub trait IntoComponentSource {
    type Source: ComponentSource;

    fn into(self) -> Self::Source;
}

pub struct Soa<T> {
    vecs: T,
}

pub struct SoaElement<T> {
    _phantom: PhantomData<T>,
    ptr: *mut T,
    len: usize,
    capacity: usize,
}

unsafe impl<T> Send for SoaElement<T> {}
unsafe impl<T> Sync for SoaElement<T> {}

impl<T> Drop for SoaElement<T> {
    fn drop(&mut self) {
        unsafe {
            // reconstruct the original vector, but with length set to the remaining elements
            let _ = Vec::from_raw_parts(self.ptr, self.len, self.capacity);
        }
    }
}

pub trait IntoSoa {
    type Source;
    fn into_soa(self) -> Self::Source;
}

pub struct Aos<T, Iter> {
    _phantom: PhantomData<T>,
    iter: Iter,
}

impl<T, Iter> Aos<T, Iter>
where
    Iter: Iterator<Item = T>,
{
    fn new(iter: Iter) -> Self {
        Self {
            iter,
            _phantom: PhantomData,
        }
    }
}

impl<I> IntoComponentSource for I
where
    I: IntoIterator,
    Aos<I::Item, I::IntoIter>: ComponentSource,
{
    type Source = Aos<I::Item, I::IntoIter>;

    fn into(self) -> Self::Source { <Self::Source>::new(self.into_iter()) }
}

macro_rules! component_source {
    ($head_ty:ident) => {
        impl_component_source!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_component_source!($head_ty, $( $tail_ty ),*);
        component_source!($( $tail_ty ),*);
    );
}

macro_rules! impl_component_source {
    ( $( $ty: ident ),* ) => {
        paste::item! {
            impl<$( $ty: Component ),*> Soa<($( SoaElement<$ty>, )*)> {
                fn validate_equal_length(vecs: &($( Vec<$ty>, )*)) -> bool {
                    #![allow(non_snake_case)]

                    let len = vecs.0.len();
                    let ($( [<$ty _vec>], )*) = vecs;
                    $(
                        if [<$ty _vec>].len() != len {
                            return false;
                        }
                    )*

                    true
                }
            }

            impl<$( $ty: Component ),*> IntoSoa for ($( Vec<$ty>, )*) {
                type Source = Soa<($( SoaElement<$ty>, )*)>;

                fn into_soa(self) -> Self::Source {
                    #![allow(non_snake_case)]

                    if !<Self::Source>::validate_equal_length(&self) {
                        panic!("all component vecs must have equal length");
                    }

                    let ($([<$ty _vec>], )*) = self;
                    Soa {
                        vecs: ($({
                            let mut [<$ty _vec>] = std::mem::ManuallyDrop::new([<$ty _vec>]);
                            SoaElement {
                                _phantom: PhantomData,
                                capacity: [<$ty _vec>].capacity(),
                                len: [<$ty _vec>].len(),
                                ptr: [<$ty _vec>].as_mut_ptr(),
                            }
                        }, )*),
                    }
                }
            }
        }

        // impl<$( $ty: Component ),*> IntoComponentSource for ($( Vec<$ty>, )*) {
        //     type Source = Soa<($( SoaElement<$ty>, )*)>;
        //     fn into(self) -> Self::Source { Soa::<($( SoaElement<$ty>, )*)>::new(self) }
        // }

        impl<$( $ty ),*> IntoComponentSource for Soa<($( SoaElement<$ty>, )*)>
        where
            Soa<($( SoaElement<$ty>, )*)>: ComponentSource
        {
            type Source = Self;
            fn into(self) -> Self::Source { self }
        }

        impl<$( $ty: Component ),*> LayoutFilter for Soa<($( SoaElement<$ty>, )*)> {
            fn matches_layout(
                &self,
                components: &[ComponentTypeId],
                _: &[TagTypeId],
            ) -> Option<bool> {
                let types = &[$( ComponentTypeId::of::<$ty>() ),*];
                Some(components.len() == types.len() && types.iter().all(|t| components.contains(t)))
            }
        }

        impl<$( $ty: Component ),*> ComponentSource for Soa<($( SoaElement<$ty>, )*)> {
            fn tailor_layout(&mut self, layout: &mut EntityTypeLayout) {
                $(
                    layout.register_component::<$ty>();
                )*
            }

            paste::item! {
                fn push_components(
                    &mut self,
                    chunk: &mut ChunkWriter,
                    mut entities: impl Iterator<Item = Entity>,
                ) -> bool {
                    #![allow(unused_variables)]
                    #![allow(non_snake_case)]

                    let len = self.vecs.0.len;
                    let to_move = std::cmp::min(len, chunk.space());
                    for _ in 0..to_move {
                        chunk.push(entities.next().unwrap());
                    }

                    let start = len - to_move;
                    let ($( [<$ty _vec>], )*) = &mut self.vecs;

                    $(
                        let type_id = ComponentTypeId::of::<$ty>();
                        let target = chunk.claim_components(type_id).unwrap();
                        unsafe {
                            let source = std::slice::from_raw_parts([<$ty _vec>].ptr.add(start), to_move);
                            target.push(source);
                            [<$ty _vec>].len -= to_move;
                        }
                    )*

                    self.vecs.0.len == 0
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.vecs.0.len, Some(self.vecs.0.len))
            }
        }

        impl<Iter, $( $ty: Component ),*> IntoComponentSource for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>,
            Aos<($( $ty, )*), Iter>: ComponentSource
        {
            type Source = Self;
            fn into(self) -> Self::Source { self }
        }

        impl<Iter, $( $ty: Component ),*> LayoutFilter for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>
        {
            fn matches_layout(
                &self,
                components: &[ComponentTypeId],
                _: &[TagTypeId],
            ) -> Option<bool> {
                let types = &[$( ComponentTypeId::of::<$ty>() ),*];
                Some(components.len() == types.len() && types.iter().all(|t| components.contains(t)))
            }
        }

        impl<Iter, $( $ty: Component ),*> ComponentSource for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>
        {
            fn tailor_layout(&mut self, layout: &mut EntityTypeLayout) {
                $(
                    layout.register_component::<$ty>();
                )*
            }

            paste::item! {
                fn push_components(
                    &mut self,
                    chunk: &mut ChunkWriter,
                    mut entities: impl Iterator<Item = Entity>,
                ) -> bool {
                    #![allow(non_snake_case)]

                    $(
                        let [<$ty _target>] = chunk.claim_components(ComponentTypeId::of::<$ty>()).unwrap();
                    )*

                    loop {
                        if (chunk.space() == 0) {
                            return match self.size_hint().1 {
                                Some(0) => true,
                                _ => false,
                            };
                        }

                        match self.iter.next() {
                            Some(($( $ty, )*)) => {
                                let entity = entities.next().unwrap();
                                chunk.push(entity);

                                $(
                                    unsafe {
                                        let slice = [$ty];
                                        [<$ty _target>].push(&slice);
                                        std::mem::forget(slice);
                                    }
                                )*
                            },
                            None => return true,
                        }
                    }
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.iter.size_hint()
            }
        }
    };
}

component_source!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(test)]
mod test {
    use super::*;
    use crate::universe::Universe;

    #[test]
    fn create() { let _ = World::default(); }

    #[test]
    fn create_in_universe() {
        let universe = Universe::new();
        let _ = universe.create_world();
    }

    #[test]
    fn extend_soa() {
        let mut world = World::default();
        let entities = world.extend(
            (1usize, false),
            (vec![1usize, 2usize, 3usize], vec![true, true, false]).into_soa(),
        );
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_soa_zerosize_tag() {
        let mut world = World::default();
        let entities = world.extend(
            ((), false),
            (vec![1usize, 2usize, 3usize], vec![true, true, false]).into_soa(),
        );
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_soa_zerosize_component() {
        let mut world = World::default();
        let entities = world.extend(
            (1usize, false),
            (vec![(), (), ()], vec![true, true, false]).into_soa(),
        );
        assert_eq!(entities.len(), 3);
    }

    #[test]
    #[should_panic(expected = "all component vecs must have equal length")]
    fn extend_soa_unbalanced_components() {
        let mut world = World::default();
        let _ = world.extend(
            (1usize, false),
            (vec![1usize, 2usize], vec![true, true, false]).into_soa(),
        );
    }

    #[test]
    #[should_panic(
        expected = "only one component of a given type may be attached to a single entity"
    )]
    fn extend_soa_duplicate_components() {
        let mut world = World::default();
        let _ = world.extend(
            (1usize, false),
            (vec![1usize, 2usize, 3usize], vec![1usize, 2usize, 3usize]).into_soa(),
        );
    }

    #[test]
    #[should_panic(expected = "only one tag of a given type may be attached to a single entity")]
    fn extend_soa_duplicate_tags() {
        let mut world = World::default();
        let _ = world.extend(
            (1usize, 1usize),
            (vec![1usize, 2usize, 3usize], vec![true, true, false]).into_soa(),
        );
    }

    #[test]
    fn extend_aos() {
        let mut world = World::default();
        let entities = world.extend(
            (1usize, false),
            vec![(1usize, true), (2usize, true), (3usize, false)],
        );
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_aos_zerosize_tag() {
        let mut world = World::default();
        let entities = world.extend(
            ((), false),
            vec![(1usize, true), (2usize, true), (3usize, false)],
        );
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_aos_zerosize_component() {
        let mut world = World::default();
        let entities = world.extend((1usize, false), vec![((), true), ((), true), ((), false)]);
        assert_eq!(entities.len(), 3);
    }

    #[test]
    #[should_panic(
        expected = "only one component of a given type may be attached to a single entity"
    )]
    fn extend_aos_duplicate_components() {
        let mut world = World::default();
        let _ = world.extend(
            (1usize, false),
            vec![(1usize, 1usize), (2usize, 2usize), (3usize, 3usize)],
        );
    }

    #[test]
    #[should_panic(expected = "only one tag of a given type may be attached to a single entity")]
    fn extend_aos_duplicate_tags() {
        let mut world = World::default();
        let _ = world.extend(
            (1usize, 1usize),
            vec![(1usize, true), (2usize, true), (3usize, false)],
        );
    }

    #[test]
    fn remove() {
        let mut world = World::default();
        let entities: Vec<_> = world
            .extend(
                (1usize, false),
                vec![(1usize, true), (2usize, true), (3usize, false)],
            )
            .iter()
            .copied()
            .collect();

        assert_eq!(world.len(), 3);
        assert!(world.remove(entities[0]));
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn remove_repeat() {
        let mut world = World::default();
        let entities: Vec<_> = world
            .extend(
                (1usize, false),
                vec![(1usize, true), (2usize, true), (3usize, false)],
            )
            .iter()
            .copied()
            .collect();

        assert_eq!(world.len(), 3);
        assert!(world.remove(entities[0]));
        assert_eq!(world.len(), 2);
        assert_eq!(world.remove(entities[0]), false);
        assert_eq!(world.len(), 2);
    }
}
