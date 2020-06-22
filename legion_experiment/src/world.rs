use crate::entity::{Entity, EntityAllocator, EntityLocation, LocationMap};
use crate::insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource};
use crate::{
    query::{filter::EntityFilter, view::View, Query},
    storage::{
        archetype::{Archetype, ArchetypeIndex, EntityLayout},
        component::ComponentTypeId,
        group::{Group, GroupDef},
        index::SearchIndex,
        ComponentIndex, Components, PackOptions,
    },
    subworld::{ComponentAccess, SubWorld},
};
use bit_set::BitSet;
use itertools::Itertools;
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Default)]
pub struct Universe {
    entity_allocator: EntityAllocator,
}

impl Universe {
    pub fn new() -> Self { Self::default() }

    pub fn sharded(n: u64, of: u64) -> Self {
        Self {
            entity_allocator: EntityAllocator::new(n, of),
        }
    }

    pub fn create_world(&self) -> World { self.create_world_with_options(WorldOptions::default()) }

    pub fn create_world_with_options(&self, options: WorldOptions) -> World {
        World {
            entity_allocator: self.entity_allocator.clone(),
            ..World::with_options(options)
        }
    }
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
pub struct WorldId(u64);
static WORLD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

impl WorldId {
    fn next() -> Self { WorldId(WORLD_ID_COUNTER.fetch_add(1, Ordering::Relaxed)) }
}

impl Default for WorldId {
    fn default() -> Self { Self::next() }
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
    epoch: u64,
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
            id: WorldId::next(),
            index: SearchIndex::default(),
            components: Components::default(),
            groups: options.groups,
            group_members,
            archetypes: Vec::default(),
            entities: LocationMap::default(),
            entity_allocator: EntityAllocator::default(),
            allocation_buffer: Vec::default(),
            epoch: 0,
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
        let mut writer = ArchetypeWriter::new(
            self.epoch,
            arch_index,
            archetype,
            self.components.get_multi_mut(),
        );
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
                storage.swap_remove(self.epoch, arch_index, component_index.0);
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

    pub fn pack(&mut self, options: PackOptions) {
        self.components.pack(&options, self.epoch);
        self.epoch += 1;
    }

    pub(crate) fn components(&self) -> &Components { &self.components }

    pub(crate) fn components_mut(&mut self) -> &mut Components { &mut self.components }

    pub(crate) fn archetypes(&self) -> &[Archetype] { &self.archetypes }

    pub(crate) fn epoch(&self) -> u64 { self.epoch }

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
                storage.move_component(self.epoch, ArchetypeIndex(from), idx, ArchetypeIndex(to));
            } else {
                storage.swap_remove(self.epoch, ArchetypeIndex(from), idx);
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

    fn insert_archetype(&mut self, layout: EntityLayout) -> ArchetypeIndex {
        // create and insert new archetype
        self.index.push(&layout);
        self.archetypes.push(Archetype::new(layout));
        let arch_index = ArchetypeIndex(self.archetypes.len() as u32 - 1);
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
            self.epoch,
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
    epoch: u64,
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
        epoch: u64,
        allowed_archetypes: Option<&'a BitSet>,
    ) -> Self {
        Self {
            id,
            index,
            components,
            archetypes,
            groups,
            group_members,
            epoch,
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

    pub fn epoch(&self) -> u64 { self.epoch }

    pub fn group(&self, type_id: ComponentTypeId) -> Option<(usize, &'a Group)> {
        self.group_members
            .get(&type_id)
            .map(|i| (*i, self.groups.get(*i).unwrap()))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::insert::IntoSoa;

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
}
