//! Contains types related to the [World](struct.World.html) entity collection.

use crate::entity::{Entity, EntityAllocator, EntityHasher, EntityLocation, LocationMap};
use crate::insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource};
use crate::{
    event::{EventSender, Subscriber, Subscribers},
    query::{
        filter::{any, EntityFilter, LayoutFilter},
        view::View,
        Query,
    },
    storage::{
        Archetype, ArchetypeIndex, Component, ComponentIndex, ComponentTypeId, Components,
        EntityLayout, Group, GroupDef, PackOptions, SearchIndex, UnknownComponentStorage,
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

/// Unique identifier for a universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniverseId(u64);
static UNIVERSE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl UniverseId {
    fn next() -> Self { UniverseId(UNIVERSE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)) }
}

impl Default for UniverseId {
    fn default() -> Self { Self::next() }
}

/// A Universe defines an [entity](../entity/struct.Entity.html) ID address space which can be shared between multiple [Worlds](struct.World.html).
///
/// A universe can be sharded to divide the entity address space between multiple universes. This can be useful in cases where entities
/// need to be allocated in two worlds which cannot share a single runtime universe; for example when running a network application across
/// multiple devices, or when mixing runtime created entities with offline serialized entities. Entities allocated in different shards of
/// a sharded universe are guaranteed to have unique entity IDs across both worlds.
///
/// Sharding a universe splits the address space into _n_ shards, where each universe is assigned a unique index from among the shards:
///
/// ```
/// # use legion::*;
/// // create universe in shard 7 of 9
/// let universe = Universe::sharded(7, 9);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Universe {
    id: UniverseId,
    entity_allocator: EntityAllocator,
}

impl Universe {
    /// Creates a new universe across the entire entity address space.
    pub fn new() -> Self { Self::default() }

    /// Creates a new universe with a sharded entity address space.
    /// `n` represents the index of the shard from a set of `of` shards.
    pub fn sharded(n: u64, of: u64) -> Self {
        Self {
            id: UniverseId::next(),
            entity_allocator: EntityAllocator::new(n, of),
        }
    }

    /// Creates a new [World](struct.World.html) in this universe with default options.
    pub fn create_world(&self) -> World { self.create_world_with_options(WorldOptions::default()) }

    /// Creates a new [World](struct.World.html) in this universe.
    pub fn create_world_with_options(&self, options: WorldOptions) -> World {
        World {
            id: WorldId::next(self.id),
            entity_allocator: self.entity_allocator.clone(),
            ..World::with_options(options)
        }
    }

    pub(crate) fn entity_allocator(&self) -> &EntityAllocator { &self.entity_allocator }
}

/// Error type representing a failure to aquire a storage accessor.
#[derive(Debug)]
pub struct ComponentAccessError;

/// The `EntityStore` trait abstracts access to entity data as required by queries for
/// both [World](struct.World.html) and [SubWorld](../subworld/struct.SubWorld.html)
pub trait EntityStore {
    /// Returns the world's unique ID.
    fn id(&self) -> WorldId;

    /// Returns an entity entry which can be used to access entity metadata and components.
    fn entry_ref(&self, entity: Entity) -> Option<crate::entry::EntryRef>;

    /// Returns a mutable entity entry which can be used to access entity metadata and components.
    fn entry_mut(&mut self, entity: Entity) -> Option<crate::entry::EntryMut>;

    /// Returns a component storage accessor for component types declared in the specified [View](../query/view/trait.View.html).
    fn get_component_storage<V: for<'b> View<'b>>(
        &self,
    ) -> Result<StorageAccessor, ComponentAccessError>;
}

/// Unique identifier for a [world](struct.World.html).
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

/// Describes configuration options for the creation of a new [world](struct.World.html).
#[derive(Default)]
pub struct WorldOptions {
    /// A vector of component [groups](../storage/group/struct.Group.html) to provide
    /// layout hints for query optimization.
    pub groups: Vec<GroupDef>,
}

/// A container of entities.
///
/// Each entity stored inside a world is uniquely identified by an [Entity](../entity/struct.Entity.html) ID
/// and may have an arbitrary collection of [components](../storage/component/trait.Component.html) attached.
///
/// The entities in a world may be efficiently searched and iterated via [queries](../query/index.html).
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
    subscribers: Subscribers,
}

impl Default for World {
    fn default() -> Self { Self::with_options(WorldOptions::default()) }
}

impl World {
    /// Creates a new world in its own [universe](struct.Universe.html) with default [options](struct.WorldOptions.html).
    pub fn new() -> Self { Self::default() }

    /// Creates a new world in its own [universe](struct.Universe.html).
    pub fn with_options(options: WorldOptions) -> Self {
        let groups: Vec<Group> = options.groups.into_iter().map(|def| def.into()).collect();
        let mut group_members = HashMap::default();
        for (i, group) in groups.iter().enumerate() {
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
            groups,
            group_members,
            archetypes: Vec::default(),
            entities: LocationMap::default(),
            entity_allocator: EntityAllocator::default(),
            allocation_buffer: Vec::default(),
            subscribers: Subscribers::default(),
        }
    }

    /// Returns the world's unique ID.
    pub fn id(&self) -> WorldId { self.id }

    /// Returns the number of entities in the world.
    pub fn len(&self) -> usize { self.entities.len() }

    /// Returns `true` if the world contains no entities.
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Returns `true` if the world contains an entity with the given ID.
    pub fn contains(&self, entity: Entity) -> bool { self.entities.contains(entity) }

    /// Appends a new entity to the world. Returns the ID of the new entity.
    /// `components` should be a tuple of components to attach to the entity.
    ///
    /// # Examples
    ///
    /// Pushing an entity with three components:
    /// ```
    /// # use legion::*;
    /// let mut world = World::new();
    /// let _entity = world.push((1usize, false, 5.3f32));
    /// ```
    ///
    /// Pushing an entity with one component (note the tuple syntax):
    /// ```
    /// # use legion::*;
    /// let mut world = World::new();
    /// let _entity = world.push((1usize,));
    /// ```
    pub fn push<T>(&mut self, components: T) -> Entity
    where
        Option<T>: IntoComponentSource,
    {
        self.extend(Some(components))[0]
    }

    /// Appends a collection of entities to the world. Returns the IDs of the new entities.
    ///
    /// # Examples
    ///
    /// Inserting a vector of component tuples:
    /// ```
    /// # use legion::*;
    /// let mut world = World::new();
    /// let _entities = world.extend(vec![
    ///     (1usize, false, 5.3f32),
    ///     (2usize, true,  5.3f32),
    ///     (3usize, false, 5.3f32),
    /// ]);
    /// ```
    ///
    /// Inserting a tuple of component vectors:
    /// ```
    /// # use legion::*;
    /// let mut world = World::new();
    /// let _entities = world.extend(
    ///     (
    ///         vec![1usize, 2usize, 3usize],
    ///         vec![false, true, false],
    ///         vec![5.3f32, 5.3f32, 5.2f32],
    ///     ).into_soa()
    /// );
    /// ```
    /// SoA inserts require all vectors to have the same length. These inserts are faster than inserting via an iterator of tuples.
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

    /// Removes the specified entity from the world. Returns `true` if an entity was removed.
    pub fn remove(&mut self, entity: Entity) -> bool {
        let location = self.entities.remove(entity);
        if let Some(location) = location {
            let EntityLocation(arch_index, component_index) = location;
            let archetype = &mut self.archetypes[arch_index];
            archetype.swap_remove(component_index.0);
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

    /// Removes all entities from the world.
    pub fn clear(&mut self) {
        use crate::query::IntoQuery;
        let mut all = Entity::query();
        let entities = all.iter(self).copied().collect::<Vec<_>>();
        for entity in entities {
            self.remove(entity);
        }
    }

    /// Gets an [entry](../entry/struct.Entry.html) for an entity, allowing manipulation of the
    /// entity.
    ///
    /// # Examples
    ///
    /// Adding a component to an entity:
    /// ```
    /// # use legion::*;
    /// let mut world = World::new();
    /// let entity = world.push((true, 0isize));
    /// if let Some(mut entry) = world.entry(entity) {
    ///     entry.add_component(0.2f32);
    /// }
    /// ```
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

    /// Subscribes to entity [events](../event/enum.Event.html).
    pub fn subscribe<T: LayoutFilter + 'static, S: EventSender + 'static>(
        &mut self,
        sender: S,
        filter: T,
    ) {
        let subscriber = Subscriber::new(filter, sender);
        for arch in &mut self.archetypes {
            if subscriber.is_interested(arch) {
                arch.subscribe(subscriber.clone());
            }
        }
        self.subscribers.push(subscriber);
    }

    /// Packs the world's internal component storage to optimise iteration performance for
    /// [queries](../query/index.html) which match a [group](../storage/group/struct.Group.html)
    /// defined when this world was created.
    pub fn pack(&mut self, options: PackOptions) { self.components.pack(&options); }

    pub(crate) fn entity_allocator(&self) -> &EntityAllocator { &self.entity_allocator }

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
        let entity = from_arch.swap_remove(idx);
        to_arch.push(entity);
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
        let subscribers = self.subscribers.matches_layout(layout.component_types());
        self.archetypes
            .push(Archetype::new(arch_index, layout, subscribers));
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

    /// Merges the given world into this world by moving all entities out of the given world.
    pub fn move_from(
        &mut self,
        source: &mut World,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        self.merge_from(source, &any(), &mut Move, None, EntityPolicy::default())
    }

    /// Merges the given world into this world by cloning all entities from the given world via a [Duplicate](struct.Duplicate.html).
    pub fn clone_from(
        &mut self,
        source: &mut World,
        cloner: &mut Duplicate,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        self.merge_from(source, &any(), cloner, None, EntityPolicy::default())
    }

    /// Merges a world into this world.
    ///
    /// A [filter](../query/filter/trait.LayoutFilter.html) selects which entities to merge.  
    /// A [merger](trait.Merger.html) describes how to perform the merge operation.  
    /// A map of entity IDs can be provided to manually remap entity IDs from the source world before they are handled by the entity policy.  
    /// A [policy](enum.EntityPolicy.html) describes how to handle entity ID allocation and conflict resolution.
    ///
    /// If any entity IDs are remapped by the policy, their mappings will be returned in the result.
    ///
    /// More advanced operations such as component type transformations can be performed with the [Duplicate](struct.Duplicate.html) merger.
    ///
    /// # Examples
    ///
    /// Moving all entities from the source world, while preserving their IDs. This will error
    /// if the ID conflicts with IDs in the destination world.
    /// ```
    /// # use legion::*;
    /// let mut world_a = World::new();
    /// let mut world_b = World::new();
    ///
    /// let _ = world_a.merge_from(&mut world_b, &any(), &mut Move, None, EntityPolicy::Move(ConflictPolicy::Error));
    /// ```
    ///
    /// Moving all entities from the source world, while reallocating IDs from the
    /// destination world's address space.
    /// ```
    /// # use legion::*;
    /// let mut world_a = World::new();
    /// let mut world_b = World::new();
    ///
    /// let _ = world_a.merge_from(&mut world_b, &any(), &mut Move, None, EntityPolicy::Reallocate);
    /// ```
    ///
    /// Moving all entities from the source world, replacing entities in the destination
    /// with entities from the source if they conflict.
    /// ```
    /// # use legion::*;
    /// let mut world_a = World::new();
    /// let mut world_b = World::new();
    ///
    /// let _ = world_a.merge_from(&mut world_b, &any(), &mut Move, None, EntityPolicy::Move(ConflictPolicy::Replace));
    /// ```
    ///
    /// Cloning all entities from the source world, converting all `i32` components to `f64` components.
    /// ```
    /// # use legion::*;
    /// let mut world_a = World::new();
    /// let mut world_b = World::new();
    ///
    /// // any component types not registered with Duplicate will be ignored during the merge
    /// let mut merger = Duplicate::default();
    /// merger.register_copy::<isize>(); // copy is faster than clone
    /// merger.register_clone::<String>();
    /// merger.register_convert(|comp: &i32| *comp as f32);
    ///
    /// let _ = world_a.merge_from(
    ///     &mut world_b,
    ///     &any(),
    ///     &mut merger,
    ///     None,
    ///     EntityPolicy::Move(ConflictPolicy::Error),
    /// );
    /// ```
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

    /// Merges a single entity from the source world into the destination world.
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

    /// Creates a serde serializable representation of the world.
    ///
    /// A [filter](../query/filter/trait.LayoutFilter.html) selects which entities shall be serialized.  
    /// A [world serializer](../serialize/ser/trait.WorldSerializer.html) describes how components will
    /// be serialized.  
    ///
    /// As component types are not known at compile time, the world must be provided with the
    /// means to serialize each component. This is provided by the
    /// [WorldSerializer](../serialize/ser/trait.WorldSerializer.html) implementation. This implementation
    /// also describes how [ComponentTypeIDs](../storage/component/struct.ComponentTypeId.html) (which
    /// are not stable between compiles) are mapped to stable type identifiers. Components that are
    /// not known to the serializer will be omitted from the serialized output.
    ///
    /// The [Registry](../serialize/struct.Registry.html) provides a
    /// [WorldSerializer](../serialize/ser/trait.WorldSerializer.html) implementation suitable for most
    /// situations.
    ///
    /// # Examples
    ///
    /// Serializing all entities with a `Position` component to JSON.
    /// ```
    /// # use legion::*;
    /// # let world = World::new();
    /// # #[derive(serde::Serialize, serde::Deserialize)]
    /// # struct Position;
    /// // create a registry which uses strings as the external type ID
    /// let mut registry = Registry::<String>::new();
    /// registry.register::<Position>("position".to_string());
    /// registry.register::<f32>("f32".to_string());
    /// registry.register::<bool>("bool".to_string());
    ///
    /// // serialize entities with the `Position` component
    /// let json = serde_json::to_value(&world.as_serializable(component::<Position>(), &registry)).unwrap();
    /// println!("{:#}", json);
    ///
    /// // registries are also serde deserializers
    /// use serde::de::DeserializeSeed;
    /// let world: World = registry.deserialize(json).unwrap();
    /// ```
    #[cfg(feature = "serialize")]
    pub fn as_serializable<'a, F: LayoutFilter, W: crate::serialize::WorldSerializer>(
        &'a self,
        filter: F,
        world_serializer: &'a W,
    ) -> crate::serialize::SerializableWorld<'a, F, W> {
        crate::serialize::SerializableWorld::new(&self, filter, world_serializer)
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

    fn id(&self) -> WorldId { self.id }
}

/// Provides access to the archetypes and entity components contained within a world.
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
    /// Constructs a new storage accessor.
    pub(crate) fn new(
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

    /// Returns the world ID.
    pub fn id(&self) -> WorldId { self.id }

    /// Returns `true` if the given archetype is accessable from this storage accessor.
    pub fn can_access_archetype(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> bool {
        match self.allowed_archetypes {
            None => true,
            Some(archetypes) => archetypes.contains(archetype as usize),
        }
    }

    /// Returns the archetype layout index.
    pub fn layout_index(&self) -> &'a SearchIndex { self.index }

    /// Returns the component storage.
    pub fn components(&self) -> &'a Components { self.components }

    /// Returns the archetypes.
    pub fn archetypes(&self) -> &'a [Archetype] { self.archetypes }

    /// Returns group definitions.
    pub fn groups(&self) -> &'a [Group] { self.groups }

    /// Returns the group the given component belongs to.
    pub fn group(&self, type_id: ComponentTypeId) -> Option<(usize, &'a Group)> {
        self.group_members
            .get(&type_id)
            .map(|i| (*i, self.groups.get(*i).unwrap()))
    }
}

/// Describes how to merge two [worlds](struct.World.html).
pub trait Merger {
    /// Indicates if the merger prefers to merge into a new empty archetype.
    #[inline]
    fn prefers_new_archetype() -> bool { false }

    /// Calculates the destination entity layout for the given source layout.
    #[inline]
    fn convert_layout(&mut self, source_layout: EntityLayout) -> EntityLayout { source_layout }

    /// Merges an archetype from the source world into the destination world.
    fn merge_archetype(
        &mut self,
        src_entity_range: Range<usize>,
        src_entities: &mut LocationMap,
        src_arch: &mut Archetype,
        src_components: &mut Components,
        dst: &mut ArchetypeWriter,
    );
}

/// A [merger](trait.Merger.html) which moves entities from the source world into the destination.
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

            for entity in src_arch.drain() {
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
                let removed = src_arch.swap_remove(entity_index);
                let location = src_entities.remove(removed).unwrap();
                if entity_index < src_arch.entities().len() {
                    let swapped = src_arch.entities()[entity_index];
                    src_entities.set(swapped, location);
                }
            }
        }
    }
}

/// A [merger](trait.Merger.html) which clones entities from the source world into the destination,
/// potentially performing data transformations in the process.
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
    /// Creates a new duplicate merger.
    pub fn new() -> Self { Self::default() }

    /// Allows the merger to copy the given component into the destination world.
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

    /// Allows the merger to clone the given component into the destination world.
    pub fn register_clone<T: Component + Clone>(&mut self) {
        self.register_convert(|source: &T| source.clone());
    }

    /// Allows the merger to clone the given component into the destination world with a custom clone function.
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

    /// Allows the merger to clone the given component into the destination world with a custom clone function.
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

/// An error type which indicates that a world merge failed.
#[derive(Debug)]
pub enum MergeError {
    /// The source world contains IDs which conflcit with the destination.
    ConflictingEntityIDs,
    /// The source world contains IDs which lie in the destination's address space but which have
    /// not yet been allocated by the destination.
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
    use crate::{insert::IntoSoa, query::filter::any};

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
            view::{Read, Write},
            IntoQuery,
        };
        use crate::storage::GroupSource;

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
