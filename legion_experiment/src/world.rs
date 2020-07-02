use crate::entity::{Entity, EntityAllocator, EntityHasher, EntityLocation, LocationMap};
use crate::insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource};
use crate::{
    query::{
        filter::{filter_fns::any, EntityFilter, LayoutFilter},
        view::View,
        Query,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex, EntityLayout},
        component::{Component, ComponentTypeId},
        group::{Group, GroupDef},
        index::SearchIndex,
        ComponentIndex, Components, PackOptions, UnknownComponentStorage,
    },
    subworld::{ComponentAccess, SubWorld},
};
use bit_set::BitSet;
use itertools::Itertools;
use std::{
    collections::{hash_map::Entry, HashMap},
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniverseId(u64);
static UNIVERSE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl UniverseId {
    fn next() -> Self { UniverseId(UNIVERSE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)) }
}

impl Default for UniverseId {
    fn default() -> Self { Self::next() }
}

#[derive(Debug, Clone, Default)]
pub struct Universe {
    id: UniverseId,
    entity_allocator: EntityAllocator,
}

impl Universe {
    pub fn new() -> Self { Self::default() }

    pub fn sharded(n: u64, of: u64) -> Self {
        Self {
            id: UniverseId::next(),
            entity_allocator: EntityAllocator::new(n, of),
        }
    }

    pub fn create_world(&self) -> World { self.create_world_with_options(WorldOptions::default()) }

    pub fn create_world_with_options(&self, options: WorldOptions) -> World {
        World {
            id: WorldId::next(self.id),
            entity_allocator: self.entity_allocator.clone(),
            ..World::with_options(options)
        }
    }

    pub(crate) fn entity_allocator(&self) -> &EntityAllocator { &self.entity_allocator }
}

#[derive(Debug)]
pub struct ComponentAccessError;

pub trait EntityStore {
    fn entry_ref(&self, entity: Entity) -> Option<crate::entry::EntryRef>;
    fn entry_mut(&mut self, entity: Entity) -> Option<crate::entry::EntryMut>;
    fn get_component_storage<V: for<'b> View<'b>>(
        &self,
    ) -> Result<StorageAccessor, ComponentAccessError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldId(UniverseId, u64);
static WORLD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl WorldId {
    fn next(universe: UniverseId) -> Self {
        WorldId(universe, WORLD_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }
    fn universe(&self) -> UniverseId { self.0 }
}

impl Default for WorldId {
    fn default() -> Self { Self::next(UniverseId::default()) }
}

#[derive(Default)]
pub struct WorldOptions {
    pub groups: Vec<Group>,
}

#[derive(Debug)]
pub struct World {
    id: WorldId,
    index: SearchIndex,
    components: Components,
    groups: Vec<Group>,
    group_members: HashMap<ComponentTypeId, usize>,
    archetypes: Vec<Archetype>,
    entities: LocationMap,
    entity_allocator: EntityAllocator,
    allocation_buffer: Vec<Entity>,
}

impl Default for World {
    fn default() -> Self { Self::with_options(WorldOptions::default()) }
}

impl World {
    pub fn new() -> Self { Self::default() }

    pub fn with_options(options: WorldOptions) -> Self {
        let mut group_members = HashMap::default();
        for (i, group) in options.groups.iter().enumerate() {
            for comp in group.components() {
                match group_members.entry(comp) {
                    Entry::Vacant(entry) => {
                        entry.insert(i);
                    }
                    Entry::Occupied(_) => {
                        panic!("components can only belong to a single group");
                    }
                }
            }
        }
        Self {
            id: WorldId::default(),
            index: SearchIndex::default(),
            components: Components::default(),
            groups: options.groups,
            group_members,
            archetypes: Vec::default(),
            entities: LocationMap::default(),
            entity_allocator: EntityAllocator::default(),
            allocation_buffer: Vec::default(),
        }
    }

    pub fn id(&self) -> WorldId { self.id }

    pub fn entity_allocator(&self) -> &EntityAllocator { &self.entity_allocator }

    pub fn len(&self) -> usize { self.entities.len() }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn contains(&self, entity: Entity) -> bool { self.entities.contains(entity) }

    pub fn push<T>(&mut self, components: T) -> Entity
    where
        Option<T>: IntoComponentSource,
    {
        self.extend(Some(components))[0]
    }

    pub fn extend(&mut self, components: impl IntoComponentSource) -> &[Entity] {
        let mut components = components.into();

        let arch_index = self.get_archetype(&mut components);
        let archetype = &mut self.archetypes[arch_index.0 as usize];
        let mut writer =
            ArchetypeWriter::new(arch_index, archetype, self.components.get_multi_mut());
        components.push_components(&mut writer, self.entity_allocator.iter());

        let (base, entities) = writer.inserted();
        self.entities.insert(entities, arch_index, base);

        self.allocation_buffer.clear();
        self.allocation_buffer.extend_from_slice(entities);
        &self.allocation_buffer
    }

    pub fn remove(&mut self, entity: Entity) -> bool {
        let location = self.entities.remove(entity);
        if let Some(location) = location {
            let EntityLocation(arch_index, component_index) = location;
            let archetype = &mut self.archetypes[arch_index];
            archetype.entities_mut().swap_remove(component_index.0);
            for type_id in archetype.layout().component_types() {
                let storage = self.components.get_mut(*type_id).unwrap();
                storage.swap_remove(arch_index, component_index);
            }
            if component_index.0 < archetype.entities().len() {
                let swapped = archetype.entities()[component_index.0];
                self.entities.set(swapped, location);
            }

            true
        } else {
            false
        }
    }

    pub fn entry(&mut self, entity: Entity) -> Option<crate::entry::Entry> {
        self.entities
            .get(entity)
            .map(move |location| crate::entry::Entry::new(location, self))
    }

    pub(crate) unsafe fn entry_unchecked(&self, entity: Entity) -> Option<crate::entry::EntryMut> {
        self.entities.get(entity).map(|location| {
            crate::entry::EntryMut::new(
                location,
                &self.components,
                &self.archetypes,
                ComponentAccess::All,
            )
        })
    }

    pub fn pack(&mut self, options: PackOptions) { self.components.pack(&options); }

    pub(crate) fn components(&self) -> &Components { &self.components }

    pub(crate) fn components_mut(&mut self) -> &mut Components { &mut self.components }

    pub(crate) fn archetypes(&self) -> &[Archetype] { &self.archetypes }

    pub(crate) fn archetypes_mut(&mut self) -> &mut [Archetype] { &mut self.archetypes }

    pub(crate) fn entities_mut(&mut self) -> &mut LocationMap { &mut self.entities }

    pub(crate) fn groups(&self) -> &[Group] { &self.groups }

    pub(crate) unsafe fn transfer_archetype(
        &mut self,
        ArchetypeIndex(from): ArchetypeIndex,
        ArchetypeIndex(to): ArchetypeIndex,
        ComponentIndex(idx): ComponentIndex,
    ) -> ComponentIndex {
        if from == to {
            return ComponentIndex(idx);
        }

        // find archetypes
        let (from_arch, to_arch) = if from < to {
            let (a, b) = self.archetypes.split_at_mut(to as usize);
            (&mut a[from as usize], &mut b[0])
        } else {
            let (a, b) = self.archetypes.split_at_mut(from as usize);
            (&mut b[0], &mut a[to as usize])
        };

        // move entity ID
        let entity = from_arch.entities_mut().swap_remove(idx);
        to_arch.entities_mut().push(entity);
        self.entities.set(
            entity,
            EntityLocation::new(
                ArchetypeIndex(to),
                ComponentIndex(to_arch.entities().len() - 1),
            ),
        );
        if from_arch.entities().len() > idx {
            let moved = from_arch.entities()[idx];
            self.entities.set(
                moved,
                EntityLocation::new(ArchetypeIndex(from), ComponentIndex(idx)),
            );
        }

        // move components
        let from_layout = from_arch.layout();
        let to_layout = to_arch.layout();
        for type_id in from_layout.component_types() {
            let storage = self.components.get_mut(*type_id).unwrap();
            if to_layout.component_types().contains(type_id) {
                storage.move_component(
                    ArchetypeIndex(from),
                    ComponentIndex(idx),
                    ArchetypeIndex(to),
                );
            } else {
                storage.swap_remove(ArchetypeIndex(from), ComponentIndex(idx));
            }
        }

        ComponentIndex(to_arch.entities().len() - 1)
    }

    pub(crate) fn get_archetype<T: ArchetypeSource>(
        &mut self,
        components: &mut T,
    ) -> ArchetypeIndex {
        let index = self.index.search(&components.filter()).next();
        if let Some(index) = index {
            index
        } else {
            self.insert_archetype(components.layout())
        }
    }

    pub(crate) fn insert_archetype(&mut self, layout: EntityLayout) -> ArchetypeIndex {
        // create and insert new archetype
        self.index.push(&layout);
        let arch_index = ArchetypeIndex(self.archetypes.len() as u32);
        self.archetypes.push(Archetype::new(arch_index, layout));
        let archetype = &self.archetypes[self.archetypes.len() - 1];

        // find all groups which contain each component
        let groups = &mut self.groups;
        let group_members = &mut self.group_members;
        let types_by_group = archetype
            .layout()
            .component_types()
            .iter()
            .map(|type_id| {
                (
                    match group_members.entry(*type_id) {
                        Entry::Occupied(entry) => *entry.get(),
                        Entry::Vacant(entry) => {
                            // create a group for the component (by itself) if it does not already have one
                            let mut group = GroupDef::new();
                            group.add(*type_id);
                            groups.push(group.into());
                            *entry.insert(groups.len() - 1)
                        }
                    },
                    *type_id,
                )
            })
            .into_group_map();

        // insert the archetype into each component storage
        for (group_index, component_types) in types_by_group.iter() {
            let group = &mut self.groups[*group_index];
            let index = group.try_insert(arch_index, archetype);
            for type_id in component_types {
                let storage = self.components.get_or_insert_with(*type_id, || {
                    let index = archetype
                        .layout()
                        .component_types()
                        .iter()
                        .position(|t| t == type_id)
                        .unwrap();
                    archetype.layout().component_constructors()[index]()
                });
                storage.insert_archetype(arch_index, index);
            }
        }

        arch_index
    }

    /// Splits the world into two. The left world allows access only to the data declared by the view;
    /// the right world allows access to all else.
    pub fn split<T: for<'v> View<'v>>(&mut self) -> (SubWorld, SubWorld) {
        let permissions = T::requires_permissions();
        let (left, right) = ComponentAccess::All.split(permissions);

        // safety: exclusive access to world, and we have split each subworld into disjoint sections
        unsafe {
            (
                SubWorld::new_unchecked(self, left, None),
                SubWorld::new_unchecked(self, right, None),
            )
        }
    }

    /// Splits the world into two. The left world allows access only to the data declared by the query's view;
    /// the right world allows access to all else.
    pub fn split_for_query<'q, V: for<'v> View<'v>, F: EntityFilter>(
        &mut self,
        _: &'q Query<V, F>,
    ) -> (SubWorld, SubWorld) {
        self.split::<V>()
    }

    pub fn move_from(
        &mut self,
        source: &mut World,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        self.merge_from(source, &any(), &mut Move, None, EntityPolicy::default())
    }

    pub fn clone_from(
        &mut self,
        source: &mut World,
        cloner: &mut Duplicate,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        self.merge_from(source, &any(), cloner, None, EntityPolicy::default())
    }

    pub fn merge_from<F: LayoutFilter, M: Merger>(
        &mut self,
        source: &mut World,
        filter: &F,
        merger: &mut M,
        remap: Option<&HashMap<Entity, Entity, EntityHasher>>,
        policy: EntityPolicy,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        let mut entity_mappings = HashMap::default();
        let mut to_push = Vec::new();

        let shared_universe = self.id.universe() == source.id.universe();

        for src_arch in source.archetypes.iter_mut().filter(|arch| {
            filter
                .matches_layout(arch.layout().component_types())
                .is_pass()
        }) {
            let allocator_clone = self.entity_allocator.clone();
            let mut allocator = allocator_clone.iter();

            to_push.reserve(src_arch.entities().len());

            match policy {
                EntityPolicy::Move(ref conflict_policy) => {
                    for src_entity in src_arch.entities() {
                        let src_entity = remap
                            .and_then(|remap| remap.get(src_entity))
                            .unwrap_or(src_entity);

                        let owns_address =
                            self.entity_allocator.address_space_contains(*src_entity);
                        let conflicted = (!shared_universe && owns_address)
                            || self.entities.get(*src_entity).is_some();
                        let entity = if conflicted {
                            match conflict_policy {
                                ConflictPolicy::Error => {
                                    return Err(MergeError::ConflictingEntityIDs)
                                }
                                ConflictPolicy::Reallocate => {
                                    let dst_entity = allocator.next().unwrap();
                                    entity_mappings.insert(*src_entity, dst_entity);
                                    dst_entity
                                }
                                ConflictPolicy::Replace => {
                                    if owns_address && !allocator.is_allocated(*src_entity) {
                                        return Err(MergeError::InvalidFutureEntityID);
                                    }
                                    self.remove(*src_entity);
                                    *src_entity
                                }
                            }
                        } else {
                            *src_entity
                        };

                        to_push.push(entity);
                    }
                }
                EntityPolicy::Reallocate => {
                    for src_entity in src_arch.entities() {
                        let src_entity = remap
                            .and_then(|remap| remap.get(src_entity))
                            .unwrap_or(src_entity);
                        let dst_entity = allocator.next().unwrap();
                        entity_mappings.insert(*src_entity, dst_entity);
                        to_push.push(dst_entity);
                    }
                }
            }

            let layout = merger.convert_layout((**src_arch.layout()).clone());
            let dst_arch_index = if !M::prefers_new_archetype() || src_arch.entities().len() < 32 {
                self.index.search(&layout).next()
            } else {
                None
            };
            let dst_arch_index = dst_arch_index.unwrap_or_else(|| self.insert_archetype(layout));
            let dst_arch = &mut self.archetypes[dst_arch_index.0 as usize];
            let mut writer =
                ArchetypeWriter::new(dst_arch_index, dst_arch, self.components.get_multi_mut());

            for entity in to_push.drain(..) {
                writer.push(entity);
            }

            merger.merge_archetype(
                0..source.entities.len(),
                &mut source.entities,
                src_arch,
                &mut source.components,
                &mut writer,
            );

            let (base, entities) = writer.inserted();
            self.entities.insert(entities, dst_arch_index, base);
        }

        Ok(entity_mappings)
    }

    pub fn merge_from_single<M: Merger>(
        &mut self,
        source: &mut World,
        entity: Entity,
        merger: &mut M,
        remap: Option<Entity>,
        policy: EntityPolicy,
    ) -> Result<Entity, MergeError> {
        let shared_universe = self.id.universe() == source.id.universe();
        let src_location = source
            .entities
            .get(entity)
            .expect("entity not found in source world");
        let src_arch = &mut source.archetypes[src_location.archetype()];

        let allocator_clone = self.entity_allocator.clone();
        let mut allocator = allocator_clone.iter();

        let dst_entity = match policy {
            EntityPolicy::Reallocate => allocator.next().unwrap(),
            EntityPolicy::Move(ref conflict_policy) => {
                let src_entity = remap.unwrap_or(entity);
                let owns_address = self.entity_allocator.address_space_contains(src_entity);
                let conflicted =
                    (!shared_universe && owns_address) || self.entities.get(src_entity).is_some();

                if conflicted {
                    match conflict_policy {
                        ConflictPolicy::Error => return Err(MergeError::ConflictingEntityIDs),
                        ConflictPolicy::Reallocate => allocator.next().unwrap(),
                        ConflictPolicy::Replace => {
                            if owns_address && !allocator.is_allocated(src_entity) {
                                return Err(MergeError::InvalidFutureEntityID);
                            }
                            self.remove(src_entity);
                            src_entity
                        }
                    }
                } else {
                    src_entity
                }
            }
        };

        let layout = merger.convert_layout((**src_arch.layout()).clone());
        let dst_arch_index = self.insert_archetype(layout);
        let dst_arch = &mut self.archetypes[dst_arch_index.0 as usize];
        let mut writer =
            ArchetypeWriter::new(dst_arch_index, dst_arch, self.components.get_multi_mut());

        writer.push(dst_entity);

        let index = src_location.component().0;
        merger.merge_archetype(
            index..(index + 1),
            &mut source.entities,
            src_arch,
            &mut source.components,
            &mut writer,
        );

        let (base, entities) = writer.inserted();
        self.entities.insert(entities, dst_arch_index, base);

        Ok(dst_entity)
    }
}

impl EntityStore for World {
    fn entry_ref(&self, entity: Entity) -> Option<crate::entry::EntryRef> {
        self.entities.get(entity).map(|location| {
            crate::entry::EntryRef::new(
                location,
                &self.components,
                &self.archetypes,
                ComponentAccess::All,
            )
        })
    }

    fn entry_mut(&mut self, entity: Entity) -> Option<crate::entry::EntryMut> {
        // safety: we have exclusive access to the world
        unsafe { self.entry_unchecked(entity) }
    }

    fn get_component_storage<V: for<'b> crate::query::view::View<'b>>(
        &self,
    ) -> Result<StorageAccessor, ComponentAccessError> {
        Ok(StorageAccessor::new(
            self.id,
            &self.index,
            &self.components,
            &self.archetypes,
            &self.groups,
            &self.group_members,
            None,
        ))
    }
}

#[derive(Clone, Copy)]
pub struct StorageAccessor<'a> {
    id: WorldId,
    index: &'a SearchIndex,
    components: &'a Components,
    archetypes: &'a [Archetype],
    groups: &'a [Group],
    group_members: &'a HashMap<ComponentTypeId, usize>,
    allowed_archetypes: Option<&'a BitSet>,
}

impl<'a> StorageAccessor<'a> {
    pub fn new(
        id: WorldId,
        index: &'a SearchIndex,
        components: &'a Components,
        archetypes: &'a [Archetype],
        groups: &'a [Group],
        group_members: &'a HashMap<ComponentTypeId, usize>,
        allowed_archetypes: Option<&'a BitSet>,
    ) -> Self {
        Self {
            id,
            index,
            components,
            archetypes,
            groups,
            group_members,
            allowed_archetypes,
        }
    }

    pub(crate) fn with_allowed_archetypes(
        mut self,
        allowed_archetypes: Option<&'a BitSet>,
    ) -> Self {
        self.allowed_archetypes = allowed_archetypes;
        self
    }

    pub fn id(&self) -> WorldId { self.id }

    pub fn can_access_archetype(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> bool {
        match self.allowed_archetypes {
            None => true,
            Some(archetypes) => archetypes.contains(archetype as usize),
        }
    }

    pub fn layout_index(&self) -> &'a SearchIndex { self.index }

    pub fn components(&self) -> &'a Components { self.components }

    pub fn archetypes(&self) -> &'a [Archetype] { self.archetypes }

    pub fn groups(&self) -> &'a [Group] { self.groups }

    pub fn group(&self, type_id: ComponentTypeId) -> Option<(usize, &'a Group)> {
        self.group_members
            .get(&type_id)
            .map(|i| (*i, self.groups.get(*i).unwrap()))
    }
}

pub trait Merger {
    #[inline]
    fn prefers_new_archetype() -> bool { false }

    #[inline]
    fn convert_layout(&mut self, source_layout: EntityLayout) -> EntityLayout { source_layout }

    fn merge_archetype(
        &mut self,
        src_entity_range: Range<usize>,
        src_entities: &mut LocationMap,
        src_arch: &mut Archetype,
        src_components: &mut Components,
        dst: &mut ArchetypeWriter,
    );
}

pub struct Move;

impl Merger for Move {
    fn prefers_new_archetype() -> bool { true }

    fn merge_archetype(
        &mut self,
        src_entity_range: Range<usize>,
        src_entities: &mut LocationMap,
        src_arch: &mut Archetype,
        src_components: &mut Components,
        dst: &mut ArchetypeWriter,
    ) {
        if src_entity_range.len() == src_arch.entities().len() {
            // faster transfer for complete archetypes
            for component in src_arch.layout().component_types() {
                let src_storage = src_components.get_mut(*component).unwrap();
                let mut dst_storage = dst.claim_components_unknown(*component);
                dst_storage.move_archetype_from(src_arch.index(), src_storage);
            }

            for entity in src_arch.entities_mut().drain(..) {
                src_entities.remove(entity);
            }
        } else {
            // per-entity transfer for partial ranges
            for component in src_arch.layout().component_types() {
                let src_storage = src_components.get_mut(*component).unwrap();
                let mut dst_storage = dst.claim_components_unknown(*component);

                for entity_index in src_entity_range.clone().rev() {
                    dst_storage.move_component_from(
                        src_arch.index(),
                        ComponentIndex(entity_index),
                        src_storage,
                    );
                }
            }

            // swap-remove entities in the same order as we swap-removed the components
            for entity_index in src_entity_range.rev() {
                let removed = src_arch.entities_mut().swap_remove(entity_index);
                let location = src_entities.remove(removed).unwrap();
                if entity_index < src_arch.entities().len() {
                    let swapped = src_arch.entities()[entity_index];
                    src_entities.set(swapped, location);
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Duplicate {
    duplicate_fns: HashMap<
        ComponentTypeId,
        (
            ComponentTypeId,
            fn() -> Box<dyn UnknownComponentStorage>,
            Box<
                dyn FnMut(
                    Range<usize>,
                    &Archetype,
                    &mut dyn UnknownComponentStorage,
                    &mut ArchetypeWriter,
                ),
            >,
        ),
    >,
}

impl Duplicate {
    pub fn new() -> Self { Self::default() }

    pub fn register_copy<T: Component + Copy>(&mut self) {
        use crate::storage::ComponentStorage;

        let type_id = ComponentTypeId::of::<T>();
        let constructor = || Box::new(T::Storage::default()) as Box<dyn UnknownComponentStorage>;
        let convert = Box::new(
            move |src_entities: Range<usize>,
                  src_arch: &Archetype,
                  src: &mut dyn UnknownComponentStorage,
                  dst: &mut ArchetypeWriter| {
                let src = src.downcast_ref::<T::Storage>().unwrap();
                let mut dst = dst.claim_components::<T>();

                let src_slice = &src.get(src_arch.index()).unwrap().into_slice()[src_entities];
                unsafe { dst.extend_memcopy(src_slice.as_ptr(), src_slice.len()) };
            },
        );

        self.duplicate_fns
            .insert(type_id, (type_id, constructor, convert));
    }

    pub fn register_clone<T: Component + Clone>(&mut self) {
        self.register_convert(|source: &T| source.clone());
    }

    pub fn register_convert<
        Source: Component,
        Target: Component,
        F: FnMut(&Source) -> Target + 'static,
    >(
        &mut self,
        mut convert: F,
    ) {
        use crate::storage::ComponentStorage;

        let source_type = ComponentTypeId::of::<Source>();
        let dest_type = ComponentTypeId::of::<Target>();
        let constructor =
            || Box::new(Target::Storage::default()) as Box<dyn UnknownComponentStorage>;
        let convert = Box::new(
            move |src_entities: Range<usize>,
                  src_arch: &Archetype,
                  src: &mut dyn UnknownComponentStorage,
                  dst: &mut ArchetypeWriter| {
                let src = src.downcast_ref::<Source::Storage>().unwrap();
                let mut dst = dst.claim_components::<Target>();

                let src_slice = &src.get(src_arch.index()).unwrap().into_slice()[src_entities];
                dst.ensure_capacity(src_slice.len());
                for component in src_slice {
                    let component = convert(component);

                    unsafe {
                        dst.extend_memcopy(&component as *const Target, 1);
                        std::mem::forget(component);
                    }
                }
            },
        );

        self.duplicate_fns
            .insert(source_type, (dest_type, constructor, convert));
    }

    pub fn register_convert_raw(
        &mut self,
        src_type: ComponentTypeId,
        dst_type: ComponentTypeId,
        constructor: fn() -> Box<dyn UnknownComponentStorage>,
        duplicate_fn: Box<
            dyn FnMut(
                Range<usize>,
                &Archetype,
                &mut dyn UnknownComponentStorage,
                &mut ArchetypeWriter,
            ),
        >,
    ) {
        self.duplicate_fns
            .insert(src_type, (dst_type, constructor, duplicate_fn));
    }
}

impl Merger for Duplicate {
    fn convert_layout(&mut self, source_layout: EntityLayout) -> EntityLayout {
        let mut layout = EntityLayout::new();
        for src_type in source_layout.component_types() {
            if let Some((dst_type, constructor, _)) = self.duplicate_fns.get(src_type) {
                unsafe { layout.register_component_raw(*dst_type, *constructor) };
            }
        }

        layout
    }

    fn merge_archetype(
        &mut self,
        src_entity_range: Range<usize>,
        _: &mut LocationMap,
        src_arch: &mut Archetype,
        src_components: &mut Components,
        dst: &mut ArchetypeWriter,
    ) {
        for src_type in src_arch.layout().component_types() {
            if let Some((_, _, convert)) = self.duplicate_fns.get_mut(src_type) {
                let src_storage = src_components.get_mut(*src_type).unwrap();
                convert(src_entity_range.clone(), src_arch, src_storage, dst);
            }
        }
    }
}

#[derive(Debug)]
pub enum MergeError {
    ConflictingEntityIDs,
    InvalidFutureEntityID,
}

/// Determines how world merges handle entity IDs.
pub enum EntityPolicy {
    /// Entity IDs will be moved from the source world into the destination.
    Move(ConflictPolicy),
    /// Entity IDs will be re-allocated in the destination world.
    Reallocate,
}

impl Default for EntityPolicy {
    fn default() -> Self { Self::Move(ConflictPolicy::default()) }
}

/// Determines how world merges handle entity ID conflicts.
pub enum ConflictPolicy {
    /// Return an error result.
    Error,
    /// Keep the ID, replacing any existing entity in the destination with the source entity.
    Replace,
    /// Allocate a new ID for the source entity, leaving any existing entities in the destination as-is.
    Reallocate,
}

impl Default for ConflictPolicy {
    fn default() -> Self { Self::Error }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{insert::IntoSoa, query::filter::filter_fns::any};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Rot(f32, f32, f32);

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
        let entities =
            world.extend((vec![1usize, 2usize, 3usize], vec![true, true, false]).into_soa());
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_soa_zerosize_component() {
        let mut world = World::default();
        let entities = world.extend((vec![(), (), ()], vec![true, true, false]).into_soa());
        assert_eq!(entities.len(), 3);
    }

    #[test]
    #[should_panic(expected = "all component vecs must have equal length")]
    fn extend_soa_unbalanced_components() {
        let mut world = World::default();
        let _ = world.extend((vec![1usize, 2usize], vec![true, true, false]).into_soa());
    }

    #[test]
    #[should_panic(
        expected = "only one component of a given type may be attached to a single entity"
    )]
    fn extend_soa_duplicate_components() {
        let mut world = World::default();
        let _ =
            world.extend((vec![1usize, 2usize, 3usize], vec![1usize, 2usize, 3usize]).into_soa());
    }

    #[test]
    fn extend_aos() {
        let mut world = World::default();
        let entities = world.extend(vec![(1usize, true), (2usize, true), (3usize, false)]);
        assert_eq!(entities.len(), 3);
    }

    #[test]
    fn extend_aos_zerosize_component() {
        let mut world = World::default();
        let entities = world.extend(vec![((), true), ((), true), ((), false)]);
        assert_eq!(entities.len(), 3);
    }

    #[test]
    #[should_panic(
        expected = "only one component of a given type may be attached to a single entity"
    )]
    fn extend_aos_duplicate_components() {
        let mut world = World::default();
        let _ = world.extend(vec![(1usize, 1usize), (2usize, 2usize), (3usize, 3usize)]);
    }

    #[test]
    fn remove() {
        let mut world = World::default();
        let entities: Vec<_> = world
            .extend(vec![(1usize, true), (2usize, true), (3usize, false)])
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
            .extend(vec![(1usize, true), (2usize, true), (3usize, false)])
            .iter()
            .copied()
            .collect();

        assert_eq!(world.len(), 3);
        assert!(world.remove(entities[0]));
        assert_eq!(world.len(), 2);
        assert_eq!(world.remove(entities[0]), false);
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn pack() {
        use crate::query::{
            view::{read::Read, write::Write},
            IntoQuery,
        };
        use crate::storage::group::GroupSource;

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct A(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct B(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct C(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct D(f32);

        let mut world = crate::world::World::with_options(WorldOptions {
            groups: vec![<(A, B, C, D)>::to_group()],
        });

        world.extend(std::iter::repeat((A(0f32),)).take(10000));
        world.extend(std::iter::repeat((A(0f32), B(1f32))).take(10000));
        world.extend(std::iter::repeat((A(0f32), B(1f32), C(2f32))).take(10000));
        world.extend(std::iter::repeat((A(0f32), B(1f32), C(2f32), D(3f32))).take(10000));
        world.pack(PackOptions::force());

        let mut query = <(Read<A>, Write<B>)>::query();

        assert_eq!(query.iter_mut(&mut world).count(), 30000);

        let mut count = 0;
        for chunk in query.iter_chunks_mut(&mut world) {
            count += chunk.into_iter().count();
        }

        assert_eq!(count, 30000);

        let mut count = 0;
        for chunk in query.iter_chunks_mut(&mut world) {
            let (x, _) = chunk.into_components();
            count += x.into_iter().count();
        }

        assert_eq!(count, 30000);
    }

    #[test]
    fn move_from() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entity_a = a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ])[0];

        let entity_b = b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ])[0];

        b.merge_from(&mut a, &any(), &mut Move, None, EntityPolicy::default())
            .unwrap();

        assert!(a.entry(entity_a).is_none());
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );
        assert_eq!(
            *b.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
    }

    #[test]
    fn move_from_single() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entities = a
            .extend(vec![
                (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
            ])
            .to_vec();

        let entity_b = b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ])[0];

        b.merge_from_single(
            &mut a,
            entities[0],
            &mut Move,
            None,
            EntityPolicy::default(),
        )
        .unwrap();

        assert!(a.entry(entities[0]).is_none());
        assert_eq!(
            *a.entry(entities[1])
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(4., 5., 6.)
        );

        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );
        assert_eq!(
            *b.entry(entities[0])
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(1., 2., 3.)
        );
    }

    #[test]
    fn clone_from() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entity_a = a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ])[0];

        let entity_b = b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ])[0];

        let mut merger = Duplicate::default();
        merger.register_copy::<Pos>();
        merger.register_clone::<Rot>();

        b.merge_from(
            &mut a,
            &any(),
            &mut merger,
            None,
            EntityPolicy::Move(ConflictPolicy::Error),
        )
        .unwrap();

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(entity_a).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.7, 0.8, 0.9)
        );
    }

    #[test]
    fn clone_from_single() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entities = a
            .extend(vec![
                (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
            ])
            .to_vec();

        let entity_b = b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ])[0];

        let mut merger = Duplicate::default();
        merger.register_copy::<Pos>();
        merger.register_clone::<Rot>();

        b.merge_from_single(
            &mut a,
            entities[0],
            &mut merger,
            None,
            EntityPolicy::Move(ConflictPolicy::Error),
        )
        .unwrap();

        assert_eq!(
            *a.entry(entities[0])
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(entities[0])
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );

        assert_eq!(
            *a.entry(entities[0])
                .unwrap()
                .get_component::<Rot>()
                .unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(entities[0])
                .unwrap()
                .get_component::<Rot>()
                .unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.7, 0.8, 0.9)
        );

        assert!(a.entry(entities[1]).is_some());
        assert!(b.entry(entities[1]).is_none());
    }

    #[test]
    fn clone_from_convert() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entity_a = a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ])[0];

        b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ]);

        let mut merger = Duplicate::default();
        merger.register_convert::<Pos, f32, _>(|comp| comp.0 as f32);

        b.merge_from(
            &mut a,
            &any(),
            &mut merger,
            None,
            EntityPolicy::Move(ConflictPolicy::Error),
        )
        .unwrap();

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(entity_a).unwrap().get_component::<f32>().unwrap(),
            1f32
        );

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert!(b.entry(entity_a).unwrap().get_component::<Rot>().is_none());
    }

    #[test]
    fn move_conflict_reallocate() {
        let mut a = World::default();
        let mut b = World::default();

        let entity_a = a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ])[0];

        let entity_b = b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ])[0];

        let mappings = b
            .merge_from(
                &mut a,
                &any(),
                &mut Move,
                None,
                EntityPolicy::Move(ConflictPolicy::Reallocate),
            )
            .unwrap();

        assert_eq!(mappings.len(), 2);
        assert!(a.entry(entity_a).is_none());
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );
        assert_eq!(
            *b.entry(*mappings.get(&entity_a).unwrap())
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(1., 2., 3.)
        );
    }

    #[test]
    #[should_panic]
    fn move_conflict_error() {
        let mut a = World::default();
        let mut b = World::default();

        a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ]);

        b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ]);

        b.merge_from(
            &mut a,
            &any(),
            &mut Move,
            None,
            EntityPolicy::Move(ConflictPolicy::Error),
        )
        .unwrap();
    }

    #[test]
    fn move_conflict_replace() {
        let mut a = World::default();
        let mut b = World::default();

        let entity = a.extend(vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ])[0];

        b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ]);

        assert_eq!(
            *b.entry(entity).unwrap().get_component::<Pos>().unwrap(),
            Pos(7., 8., 9.)
        );

        b.merge_from(
            &mut a,
            &any(),
            &mut Move,
            None,
            EntityPolicy::Move(ConflictPolicy::Replace),
        )
        .unwrap();

        assert_eq!(
            *b.entry(entity).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
    }
}
