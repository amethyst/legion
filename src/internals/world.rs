//! Contains types related to the [World](struct.World.html) entity collection.

use super::entity::{
    Allocate, Canon, Entity, EntityHasher, EntityLocation, EntityName, LocationMap,
    ID_CLONE_MAPPINGS,
};
use super::insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource};
use super::{
    entry::{Entry, EntryMut, EntryRef},
    event::{EventSender, Subscriber, Subscribers},
    query::{
        filter::{EntityFilter, LayoutFilter},
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
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use thiserror::Error;

type MapEntry<'a, K, V> = std::collections::hash_map::Entry<'a, K, V>;

/// Unique identifier for a universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniverseId(u64);
static UNIVERSE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl UniverseId {
    fn next() -> Self {
        UniverseId(UNIVERSE_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for UniverseId {
    fn default() -> Self {
        Self::next()
    }
}

/// A Universe defines a namespace where canonical entity names will always resolve to the same
/// [Entity](../entity/struct.Entity.html).
#[derive(Debug, Clone, Default)]
pub struct Universe {
    id: UniverseId,
    canon: Arc<RwLock<Canon>>,
}

impl Universe {
    /// Creates a new universe across the entire entity address space.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new [World](struct.World.html) in this universe with default options.
    pub fn create_world(&self) -> World {
        self.create_world_with_options(WorldOptions::default())
    }

    /// Creates a new [World](struct.World.html) in this universe.
    pub fn create_world_with_options(&self, options: WorldOptions) -> World {
        World {
            id: WorldId::next(self.id),
            universe: self.clone(),
            ..World::with_options(options)
        }
    }

    /// Returns an RAII object which can deref into the universe's canon.
    pub fn canon(&self) -> RwLockReadGuard<'_, Canon> {
        self.canon.read_recursive()
    }

    /// Returns an RAII object which can deref mut into the universe's canon.
    pub fn canon_mut(&self) -> RwLockWriteGuard<'_, Canon> {
        self.canon.write()
    }
}

/// Error type representing a failure to aquire a storage accessor.
#[derive(Debug)]
pub struct ComponentAccessError;

/// The `EntityStore` trait abstracts access to entity data as required by queries for
/// both [World](struct.World.html) and [SubWorld](struct.SubWorld.html)
pub trait EntityStore {
    /// Returns the world's unique ID.
    fn id(&self) -> WorldId;

    /// Returns an entity entry which can be used to access entity metadata and components.
    fn entry_ref(&self, entity: Entity) -> Option<EntryRef>;

    /// Returns a mutable entity entry which can be used to access entity metadata and components.
    fn entry_mut(&mut self, entity: Entity) -> Option<EntryMut>;

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
    fn universe(&self) -> UniverseId {
        self.0
    }
}

impl Default for WorldId {
    fn default() -> Self {
        Self::next(UniverseId::default())
    }
}

/// Describes configuration options for the creation of a new [world](struct.World.html).
#[derive(Default)]
pub struct WorldOptions {
    /// A vector of component [groups](../storage/struct.Group.html) to provide
    /// layout hints for query optimization.
    pub groups: Vec<GroupDef>,
}

/// A container of entities.
///
/// Each entity stored inside a world is uniquely identified by an [Entity](struct.Entity.html) ID
/// and may have an arbitrary collection of [components](../storage/trait.Component.html) attached.
///
/// The entities in a world may be efficiently searched and iterated via [queries](../query/index.html).
#[derive(Debug)]
pub struct World {
    universe: Universe,
    id: WorldId,
    index: SearchIndex,
    components: Components,
    groups: Vec<Group>,
    group_members: HashMap<ComponentTypeId, usize>,
    archetypes: Vec<Archetype>,
    entities: LocationMap,
    allocation_buffer: Vec<Entity>,
    subscribers: Subscribers,
}

impl Default for World {
    fn default() -> Self {
        Self::with_options(WorldOptions::default())
    }
}

impl World {
    /// Creates a new world in its own [universe](struct.Universe.html) with default [options](struct.WorldOptions.html).
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new world in its own [universe](struct.Universe.html).
    pub fn with_options(options: WorldOptions) -> Self {
        let groups: Vec<Group> = options.groups.into_iter().map(|def| def.into()).collect();
        let mut group_members = HashMap::default();
        for (i, group) in groups.iter().enumerate() {
            for comp in group.components() {
                match group_members.entry(comp) {
                    MapEntry::Vacant(entry) => {
                        entry.insert(i);
                    }
                    MapEntry::Occupied(_) => {
                        panic!("components can only belong to a single group");
                    }
                }
            }
        }

        let universe = Universe::default();
        let id = WorldId::next(universe.id);

        Self {
            universe,
            id,
            index: SearchIndex::default(),
            components: Components::default(),
            groups,
            group_members,
            archetypes: Vec::default(),
            entities: LocationMap::default(),
            allocation_buffer: Vec::default(),
            subscribers: Subscribers::default(),
        }
    }

    /// Returns the world's unique ID.
    pub fn id(&self) -> WorldId {
        self.id
    }

    /// Returns the number of entities in the world.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns `true` if the world contains no entities.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the world contains an entity with the given ID.
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
    }

    /// Appends a named entity to the word, replacing any existing entity with the given canonical name.
    /// Returns the ID of the entity.
    pub fn push_named<T>(&mut self, name: &EntityName, components: T) -> Entity
    where
        Option<T>: IntoComponentSource,
    {
        let entity_id = self.universe.canon_mut().canonize_name(name);
        self.remove(entity_id);

        let mut components = <Option<T> as IntoComponentSource>::into(Some(components));

        let arch_index = self.get_archetype_for_components(&mut components);
        let archetype = &mut self.archetypes[arch_index.0 as usize];
        let mut writer =
            ArchetypeWriter::new(arch_index, archetype, self.components.get_multi_mut());
        components.push_components(&mut writer, std::iter::once(entity_id));

        let (base, entities) = writer.inserted();
        self.entities.insert(entities, arch_index, base);

        entity_id
    }

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

        let arch_index = self.get_archetype_for_components(&mut components);
        let archetype = &mut self.archetypes[arch_index.0 as usize];
        let mut writer =
            ArchetypeWriter::new(arch_index, archetype, self.components.get_multi_mut());
        components.push_components(&mut writer, Allocate::new());

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
        use crate::internals::query::IntoQuery;
        let mut all = Entity::query();
        let entities = all.iter(self).copied().collect::<Vec<_>>();
        for entity in entities {
            self.remove(entity);
        }
    }

    /// Gets an [entry](struct.Entry.html) for an entity, allowing manipulation of the
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
    pub fn entry(&mut self, entity: Entity) -> Option<Entry> {
        self.entities
            .get(entity)
            .map(move |location| Entry::new(location, self))
    }

    pub(crate) unsafe fn entry_unchecked(&self, entity: Entity) -> Option<EntryMut> {
        self.entities.get(entity).map(|location| {
            EntryMut::new(
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
    /// [queries](../query/index.html) which match a [group](../storage/struct.Group.html)
    /// defined when this world was created.
    pub fn pack(&mut self, options: PackOptions) {
        self.components.pack(&options);
    }

    /// Returns the world's universe.
    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    pub(crate) fn components(&self) -> &Components {
        &self.components
    }

    pub(crate) fn components_mut(&mut self) -> &mut Components {
        &mut self.components
    }

    pub(crate) fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    pub(crate) fn archetypes_mut(&mut self) -> &mut [Archetype] {
        &mut self.archetypes
    }

    pub(crate) fn entities_mut(&mut self) -> &mut LocationMap {
        &mut self.entities
    }

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

    pub(crate) fn get_archetype_for_components<T: ArchetypeSource>(
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

    pub(crate) fn get_or_insert_archetype(&mut self, layout: EntityLayout) -> ArchetypeIndex {
        let index = self.index.search(&layout).next();
        if let Some(index) = index {
            index
        } else {
            self.insert_archetype(layout)
        }
    }

    fn insert_archetype(&mut self, layout: EntityLayout) -> ArchetypeIndex {
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
                        MapEntry::Occupied(entry) => *entry.get(),
                        MapEntry::Vacant(entry) => {
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

    /// Merges the given world into this world by moving all entities out of the source world.
    pub fn move_from<F: LayoutFilter>(
        &mut self,
        source: &mut World,
        filter: &F,
    ) -> Result<(), MergeError> {
        // we can only merge worlds which are part of the same universe
        if self.id.universe() != source.id.universe() {
            return Err(MergeError::DifferentUniverses);
        }

        // find the archetypes in the source that we want to merge into the destination
        for src_arch in source.archetypes.iter_mut().filter(|arch| {
            filter
                .matches_layout(arch.layout().component_types())
                .is_pass()
        }) {
            // find conflicts, and remove the existing entity, to be replaced with that defined in the source
            for src_entity in src_arch.entities() {
                self.remove(*src_entity);
            }

            // find or construct the destination archetype
            let layout = &**src_arch.layout();
            let dst_arch_index = if src_arch.entities().len() < 32 {
                self.index.search(layout).next()
            } else {
                None
            };
            let dst_arch_index =
                dst_arch_index.unwrap_or_else(|| self.insert_archetype(layout.clone()));
            let dst_arch = &mut self.archetypes[dst_arch_index.0 as usize];

            // build a writer for the destination archetype
            let mut writer =
                ArchetypeWriter::new(dst_arch_index, dst_arch, self.components.get_multi_mut());

            // push entity IDs into the archetype
            for entity in src_arch.entities() {
                writer.push(*entity);
            }

            // merge components into the archetype
            for component in src_arch.layout().component_types() {
                let src_storage = source.components.get_mut(*component).unwrap();
                let mut dst_storage = writer.claim_components_unknown(*component);
                dst_storage.move_archetype_from(src_arch.index(), src_storage);
            }

            for entity in src_arch.drain() {
                source.entities.remove(entity);
            }

            // record entity locations
            let (base, entities) = writer.inserted();
            self.entities.insert(entities, dst_arch_index, base);
        }

        Ok(())
    }

    /// Clones the entities from a world into this world.
    ///
    /// A [filter](../query/trait.LayoutFilter.html) selects which entities to merge.  
    /// A [merger](trait.Merger.html) describes how to perform the merge operation.  
    ///
    /// If any entity IDs are remapped by the policy, their mappings will be returned in the result.
    ///
    /// More advanced operations such as component type transformations can be performed with the [Duplicate](struct.Duplicate.html) merger.
    ///
    /// # Examples
    ///
    /// Cloning all entities from the source world, converting all `i32` components to `f64` components.
    /// ```
    /// # use legion::*;
    /// # use legion::world::Duplicate;
    /// let mut world_a = World::new();
    /// let mut world_b = World::new();
    ///
    /// // any component types not registered with Duplicate will be ignored during the merge
    /// let mut merger = Duplicate::default();
    /// merger.register_copy::<isize>(); // copy is faster than clone
    /// merger.register_clone::<String>();
    /// merger.register_convert(|comp: &i32| *comp as f32);
    ///
    /// let _ = world_a.clone_from(&world_b, &any(), &mut merger);
    /// ```
    pub fn clone_from<F: LayoutFilter, M: Merger>(
        &mut self,
        source: &World,
        filter: &F,
        merger: &mut M,
    ) -> Result<HashMap<Entity, Entity, EntityHasher>, MergeError> {
        // we can only merge worlds which are part of the same universe
        if self.id.universe() != source.id.universe() {
            return Err(MergeError::DifferentUniverses);
        }

        let universe = self.universe.clone();
        let mut canon = universe.canon_mut();
        let mut allocator = Allocate::new();
        let mut reallocated = HashMap::default();

        // assign destination IDs
        for src_arch in source.archetypes.iter().filter(|arch| {
            filter
                .matches_layout(arch.layout().component_types())
                .is_pass()
        }) {
            // find conflicts, and remove the existing entity, to be replaced with that defined in the source
            for src_entity in src_arch.entities() {
                let dst_entity = merger.assign_id(*src_entity, &mut allocator, &mut *canon);
                self.remove(dst_entity);
                reallocated.insert(*src_entity, dst_entity);
            }
        }

        let mut reallocated = Some(reallocated);
        let mut mappings = match merger.entity_map() {
            EntityRewrite::Auto(Some(mut overrides)) => {
                for (a, b) in reallocated.as_ref().unwrap().iter() {
                    overrides.entry(*a).or_insert(*b);
                }
                overrides
            }
            EntityRewrite::Auto(None) => reallocated.take().unwrap(),
            EntityRewrite::Explicit(overrides) => overrides,
        };

        // set the entity mappings as context for Entity::clone
        ID_CLONE_MAPPINGS.with(|cell| {
            std::mem::swap(&mut *cell.borrow_mut(), &mut mappings);
        });

        // clone entities
        for src_arch in source.archetypes.iter().filter(|arch| {
            filter
                .matches_layout(arch.layout().component_types())
                .is_pass()
        }) {
            // construct the destination entity layout
            let layout = merger.convert_layout((**src_arch.layout()).clone());

            // find or construct the destination archetype
            let dst_arch_index = if !M::prefers_new_archetype() || src_arch.entities().len() < 32 {
                self.index.search(&layout).next()
            } else {
                None
            };
            let dst_arch_index = dst_arch_index.unwrap_or_else(|| self.insert_archetype(layout));
            let dst_arch = &mut self.archetypes[dst_arch_index.0 as usize];

            // build a writer for the destination archetype
            let mut writer =
                ArchetypeWriter::new(dst_arch_index, dst_arch, self.components.get_multi_mut());

            // push entity IDs into the archetype
            ID_CLONE_MAPPINGS.with(|cell| {
                let map = cell.borrow();
                for entity in src_arch.entities() {
                    let entity = map.get(entity).unwrap_or(entity);
                    writer.push(*entity);
                }
            });

            // merge components into the archetype
            merger.merge_archetype(
                0..src_arch.entities().len(),
                src_arch,
                &source.components,
                &mut writer,
            );

            // record entity locations
            let (base, entities) = writer.inserted();
            self.entities.insert(entities, dst_arch_index, base);
        }

        let reallocated = reallocated.unwrap_or_else(|| {
            // switch the map context back to recover our hashmap
            ID_CLONE_MAPPINGS.with(|cell| {
                std::mem::swap(&mut *cell.borrow_mut(), &mut mappings);
            });
            mappings
        });

        Ok(reallocated)
    }

    /// Clones a single entity from the source world into the destination world.
    pub fn clone_from_single<M: Merger>(
        &mut self,
        source: &World,
        entity: Entity,
        merger: &mut M,
    ) -> Result<Entity, MergeError> {
        // we can only merge worlds which are part of the same universe
        if self.id.universe() != source.id.universe() {
            return Err(MergeError::DifferentUniverses);
        }

        let mut allocator = Allocate::new();
        let universe = self.universe.clone();
        let mut canon = universe.canon_mut();

        // determine the destination ID
        let dst_entity = merger.assign_id(entity, &mut allocator, &mut *canon);

        // find conflicts, and remove the existing entity, to be replaced with that defined in the source
        self.remove(dst_entity);

        // find the source
        let src_location = source
            .entities
            .get(entity)
            .expect("entity not found in source world");
        let src_arch = &source.archetypes[src_location.archetype()];

        // construct the destination entity layout
        let layout = merger.convert_layout((**src_arch.layout()).clone());

        // find or construct the destination archetype
        let dst_arch_index = self.insert_archetype(layout);
        let dst_arch = &mut self.archetypes[dst_arch_index.0 as usize];

        // build a writer for the destination archetype
        let mut writer =
            ArchetypeWriter::new(dst_arch_index, dst_arch, self.components.get_multi_mut());

        // push the entity ID into the archetype
        writer.push(dst_entity);

        let mut mappings = match merger.entity_map() {
            EntityRewrite::Auto(Some(mut overrides)) => {
                overrides.entry(entity).or_insert(dst_entity);
                overrides
            }
            EntityRewrite::Auto(None) => {
                let mut map = HashMap::default();
                map.insert(entity, dst_entity);
                map
            }
            EntityRewrite::Explicit(overrides) => overrides,
        };

        // set the entity mappings as context for Entity::clone
        ID_CLONE_MAPPINGS.with(|cell| {
            std::mem::swap(&mut *cell.borrow_mut(), &mut mappings);
        });

        // merge components into the archetype
        let index = src_location.component().0;
        merger.merge_archetype(
            index..(index + 1),
            src_arch,
            &source.components,
            &mut writer,
        );

        // record entity location
        let (base, entities) = writer.inserted();
        self.entities.insert(entities, dst_arch_index, base);

        ID_CLONE_MAPPINGS.with(|cell| {
            cell.borrow_mut().clear();
        });

        Ok(dst_entity)
    }

    /// Creates a serde serializable representation of the world.
    ///
    /// A [filter](../query/trait.LayoutFilter.html) selects which entities shall be serialized.  
    /// A [world serializer](../serialize/trait.WorldSerializer.html) describes how components will
    /// be serialized.  
    ///
    /// As component types are not known at compile time, the world must be provided with the
    /// means to serialize each component. This is provided by the
    /// [WorldSerializer](../serialize/trait.WorldSerializer.html) implementation. This implementation
    /// also describes how [ComponentTypeIDs](../storage/struct.ComponentTypeId.html) (which
    /// are not stable between compiles) are mapped to stable type identifiers. Components that are
    /// not known to the serializer will be omitted from the serialized output.
    ///
    /// The [Registry](../serialize/struct.Registry.html) provides a
    /// [WorldSerializer](../serialize/trait.WorldSerializer.html) implementation suitable for most
    /// situations.
    ///
    /// # Examples
    ///
    /// Serializing all entities with a `Position` component to JSON.
    /// ```
    /// # use legion::*;
    /// # let universe = Universe::new();
    /// # let world = universe.create_world();
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
    /// // registries can also be used to deserialize
    /// use serde::de::DeserializeSeed;
    /// let world: World = registry.as_deserialize(&universe).deserialize(json).unwrap();
    /// ```
    #[cfg(feature = "serialize")]
    pub fn as_serializable<
        'a,
        F: LayoutFilter,
        W: crate::internals::serialize::ser::WorldSerializer,
    >(
        &'a self,
        filter: F,
        world_serializer: &'a W,
    ) -> crate::internals::serialize::ser::SerializableWorld<'a, F, W> {
        crate::internals::serialize::ser::SerializableWorld::new(&self, filter, world_serializer)
    }
}

impl EntityStore for World {
    fn entry_ref(&self, entity: Entity) -> Option<EntryRef> {
        self.entities.get(entity).map(|location| {
            EntryRef::new(
                location,
                &self.components,
                &self.archetypes,
                ComponentAccess::All,
            )
        })
    }

    fn entry_mut(&mut self, entity: Entity) -> Option<EntryMut> {
        // safety: we have exclusive access to the world
        unsafe { self.entry_unchecked(entity) }
    }

    fn get_component_storage<V: for<'b> View<'b>>(
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

    fn id(&self) -> WorldId {
        self.id
    }
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
    pub fn id(&self) -> WorldId {
        self.id
    }

    /// Returns `true` if the given archetype is accessable from this storage accessor.
    pub fn can_access_archetype(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> bool {
        match self.allowed_archetypes {
            None => true,
            Some(archetypes) => archetypes.contains(archetype as usize),
        }
    }

    /// Returns the archetype layout index.
    pub fn layout_index(&self) -> &'a SearchIndex {
        self.index
    }

    /// Returns the component storage.
    pub fn components(&self) -> &'a Components {
        self.components
    }

    /// Returns the archetypes.
    pub fn archetypes(&self) -> &'a [Archetype] {
        self.archetypes
    }

    /// Returns group definitions.
    pub fn groups(&self) -> &'a [Group] {
        self.groups
    }

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
    fn prefers_new_archetype() -> bool {
        false
    }

    /// Indicates how the merger wishes entity IDs to be adjusted while cloning a world.
    fn entity_map(&mut self) -> EntityRewrite {
        EntityRewrite::default()
    }

    /// Returns the ID to use in the destination world when cloning the given entity.
    #[inline]
    #[allow(unused_variables)]
    fn assign_id(
        &mut self,
        existing: Entity,
        allocator: &mut Allocate,
        canon: &mut Canon,
    ) -> Entity {
        allocator.next().unwrap()
    }

    /// Calculates the destination entity layout for the given source layout.
    #[inline]
    fn convert_layout(&mut self, source_layout: EntityLayout) -> EntityLayout {
        source_layout
    }

    /// Merges an archetype from the source world into the destination world.
    fn merge_archetype(
        &mut self,
        src_entity_range: Range<usize>,
        src_arch: &Archetype,
        src_components: &Components,
        dst: &mut ArchetypeWriter,
    );
}

/// Describes how a merger wishes `Entity` references inside cloned components to be
/// rewritten.
pub enum EntityRewrite {
    /// Replace references to entities which have been cloned with the ID of their clone.
    /// May also provide a map of additional IDs to replace.
    Auto(Option<HashMap<Entity, Entity, EntityHasher>>),
    /// Replace IDs according to the given map.
    Explicit(HashMap<Entity, Entity, EntityHasher>),
}

impl Default for EntityRewrite {
    fn default() -> Self {
        Self::Auto(None)
    }
}

/// A [merger](trait.Merger.html) which clones entities from the source world into the destination,
/// potentially performing data transformations in the process.
#[derive(Default)]
#[allow(clippy::type_complexity)]
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
                    &dyn UnknownComponentStorage,
                    &mut ArchetypeWriter,
                ),
            >,
        ),
    >,
}

impl Duplicate {
    /// Creates a new duplicate merger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allows the merger to copy the given component into the destination world.
    pub fn register_copy<T: Component + Copy>(&mut self) {
        use crate::internals::storage::ComponentStorage;

        let type_id = ComponentTypeId::of::<T>();
        let constructor = || Box::new(T::Storage::default()) as Box<dyn UnknownComponentStorage>;
        let convert = Box::new(
            move |src_entities: Range<usize>,
                  src_arch: &Archetype,
                  src: &dyn UnknownComponentStorage,
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
        use crate::internals::storage::ComponentStorage;

        let source_type = ComponentTypeId::of::<Source>();
        let dest_type = ComponentTypeId::of::<Target>();
        let constructor =
            || Box::new(Target::Storage::default()) as Box<dyn UnknownComponentStorage>;
        let convert = Box::new(
            move |src_entities: Range<usize>,
                  src_arch: &Archetype,
                  src: &dyn UnknownComponentStorage,
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
            dyn FnMut(Range<usize>, &Archetype, &dyn UnknownComponentStorage, &mut ArchetypeWriter),
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
        src_arch: &Archetype,
        src_components: &Components,
        dst: &mut ArchetypeWriter,
    ) {
        for src_type in src_arch.layout().component_types() {
            if let Some((_, _, convert)) = self.duplicate_fns.get_mut(src_type) {
                let src_storage = src_components.get(*src_type).unwrap();
                convert(src_entity_range.clone(), src_arch, src_storage, dst);
            }
        }
    }
}

/// An error type which indicates that a world merge failed.
#[derive(Error, Debug, Copy, Clone, PartialEq, Hash)]
pub enum MergeError {
    /// The two worlds exist in differnce universes.
    #[error("source and destination worlds belong to different universes")]
    DifferentUniverses,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::internals::{insert::IntoSoa, query::filter::filter_fns::any};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Rot(f32, f32, f32);

    #[test]
    fn create() {
        let _ = World::default();
    }

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
        use crate::internals::{
            query::{
                view::{read::Read, write::Write},
                IntoQuery,
            },
            storage::group::GroupSource,
        };

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct A(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct B(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct C(f32);

        #[derive(Copy, Clone, Debug, PartialEq)]
        struct D(f32);

        let mut world = crate::internals::world::World::with_options(WorldOptions {
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
            count += x.iter().count();
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

        b.move_from(&mut a, &any()).unwrap();

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

        let map = b.clone_from(&a, &any(), &mut merger).unwrap();

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(map[&entity_a])
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
            *a.entry(entity_a).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(map[&entity_a])
                .unwrap()
                .get_component::<Rot>()
                .unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert_eq!(
            *b.entry(entity_b).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.7, 0.8, 0.9)
        );
    }

    #[test]
    fn clone_update_entity_refs() {
        let universe = Universe::new();
        let mut a = universe.create_world();
        let mut b = universe.create_world();

        let entity_1 = a.push((Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)));
        let entity_2 = a.push((Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6), entity_1));
        a.entry(entity_1).unwrap().add_component(entity_2);

        b.extend(vec![
            (Pos(7., 8., 9.), Rot(0.7, 0.8, 0.9)),
            (Pos(10., 11., 12.), Rot(0.10, 0.11, 0.12)),
        ]);

        let mut merger = Duplicate::default();
        merger.register_copy::<Pos>();
        merger.register_clone::<Rot>();
        merger.register_clone::<Entity>();

        let map = b.clone_from(&a, &any(), &mut merger).unwrap();

        assert_eq!(
            *b.entry(map[&entity_1])
                .unwrap()
                .get_component::<Entity>()
                .unwrap(),
            map[&entity_2]
        );
        assert_eq!(
            *b.entry(map[&entity_2])
                .unwrap()
                .get_component::<Entity>()
                .unwrap(),
            map[&entity_1]
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

        let cloned = b.clone_from_single(&a, entities[0], &mut merger).unwrap();

        assert_eq!(
            *a.entry(entities[0])
                .unwrap()
                .get_component::<Pos>()
                .unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(cloned).unwrap().get_component::<Pos>().unwrap(),
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
            *b.entry(cloned).unwrap().get_component::<Rot>().unwrap(),
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
    #[allow(clippy::float_cmp)]
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

        let map = b.clone_from(&a, &any(), &mut merger).unwrap();

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Pos>().unwrap(),
            Pos(1., 2., 3.)
        );
        assert_eq!(
            *b.entry(map[&entity_a])
                .unwrap()
                .get_component::<f32>()
                .unwrap(),
            1f32
        );

        assert_eq!(
            *a.entry(entity_a).unwrap().get_component::<Rot>().unwrap(),
            Rot(0.1, 0.2, 0.3)
        );
        assert!(b
            .entry(map[&entity_a])
            .unwrap()
            .get_component::<Rot>()
            .is_err());
    }
}
