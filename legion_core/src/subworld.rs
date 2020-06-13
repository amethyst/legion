use crate::{
    borrow::{Ref, RefMut},
    entity::Entity,
    filter::EntityFilter,
    index::ArchetypeIndex,
    prelude::Query,
    query::View,
    storage::{Component, ComponentTypeId, Storage, Tag},
    world::{EntityStore, World},
};
use bit_set::BitSet;
use derivative::Derivative;
use std::{borrow::Cow, ops::Deref};

/// Describes which archetypes are available for access.
pub enum ArchetypeAccess {
    /// All archetypes.
    All,
    /// Some archetypes.
    Some(BitSet),
}

impl ArchetypeAccess {
    pub fn is_disjoint(&self, other: &ArchetypeAccess) -> bool {
        match self {
            Self::All => false,
            Self::Some(mine) => match other {
                Self::All => false,
                Self::Some(theirs) => mine.is_disjoint(theirs),
            },
        }
    }
}

/// Describes items that are accessed via reads or writes.
#[derive(Derivative, Debug, Clone)]
#[derivative(Default(bound = ""))]
pub struct Access<T> {
    pub reads: Vec<T>,
    pub writes: Vec<T>,
}

#[derive(Clone)]
pub enum ComponentAccess<'a> {
    All,
    Allow(Cow<'a, Access<ComponentTypeId>>),
    Disallow(Cow<'a, Access<ComponentTypeId>>),
}

impl<'a> ComponentAccess<'a> {
    pub fn allows_read(&self, component: ComponentTypeId) -> bool {
        match self {
            Self::All => true,
            Self::Allow(components) => components.reads.contains(&component),
            Self::Disallow(components) => !components.reads.contains(&component),
        }
    }

    pub fn allows_write(&self, component: ComponentTypeId) -> bool {
        match self {
            Self::All => true,
            Self::Allow(components) => components.writes.contains(&component),
            Self::Disallow(components) => !components.writes.contains(&component),
        }
    }

    pub(crate) fn split(&mut self, access: Access<ComponentTypeId>) -> (Self, Self) {
        fn invert(mut access: Access<ComponentTypeId>) -> Access<ComponentTypeId> {
            // reads are now denied writes
            let original_write_len = access.writes.len();
            for read in access.reads.drain(..) {
                access.writes.push(read);
            }

            // writes are now entirely denied
            for write in access.writes.iter().take(original_write_len) {
                access.reads.push(*write);
            }

            access
        }

        fn union(
            a: &Access<ComponentTypeId>,
            mut b: Access<ComponentTypeId>,
        ) -> Access<ComponentTypeId> {
            for read in &a.reads {
                if !b.reads.contains(read) {
                    b.reads.push(*read);
                }
            }
            for write in &a.writes {
                if !b.writes.contains(write) {
                    b.writes.push(*write);
                }
            }
            b
        }

        fn subtract(
            a: &Access<ComponentTypeId>,
            b: Access<ComponentTypeId>,
        ) -> Access<ComponentTypeId> {
            let mut a = a.to_owned();
            for read in &b.reads {
                if let Some(i) = a.reads.iter().position(|t| t == read) {
                    a.reads.swap_remove(i);
                }
            }
            for write in &b.writes {
                if let Some(i) = a.writes.iter().position(|t| t == write) {
                    a.writes.swap_remove(i);
                }
            }
            a
        }

        match self {
            Self::All => (
                Self::Allow(Cow::Owned(access.clone())),
                Self::Disallow(Cow::Owned(invert(access))),
            ),
            Self::Allow(current) => {
                if access.reads.iter().any(|t| !current.reads.contains(t))
                    || access.writes.iter().any(|t| !current.writes.contains(t))
                {
                    panic!("view accesses components unavailable in this world");
                }

                (
                    Self::Allow(Cow::Owned(access.clone())),
                    Self::Allow(Cow::Owned(subtract(current, access))),
                )
            }
            Self::Disallow(current) => {
                if access.reads.iter().any(|t| current.reads.contains(t))
                    || access.writes.iter().any(|t| current.writes.contains(t))
                {
                    panic!("view accesses components unavailable in this world");
                }

                (
                    Self::Allow(Cow::Owned(access.clone())),
                    Self::Disallow(Cow::Owned(union(current, invert(access)))),
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct ComponentAccessError;

#[derive(Clone)]
pub struct StorageAccessor<'a> {
    storage: &'a Storage,
    archetypes: Option<&'a BitSet>,
}

impl<'a> StorageAccessor<'a> {
    pub fn new(storage: &'a Storage, archetypes: Option<&'a BitSet>) -> Self {
        Self {
            storage,
            archetypes,
        }
    }

    pub fn can_access_archetype(&self, ArchetypeIndex(archetype): ArchetypeIndex) -> bool {
        match self.archetypes {
            None => true,
            Some(archetypes) => archetypes.contains(archetype),
        }
    }

    pub fn inner(&self) -> &'a Storage { self.storage }

    pub fn into_inner(self) -> &'a Storage { self.storage }
}

impl<'a> Deref for StorageAccessor<'a> {
    type Target = Storage;
    fn deref(&self) -> &Self::Target { self.storage }
}

/// Provides access to a subset of the entities of a `World`.
#[derive(Clone)]
pub struct SubWorld<'a> {
    pub(crate) world: &'a World,
    pub(crate) components: ComponentAccess<'a>,
    pub(crate) archetypes: Option<&'a BitSet>,
}

impl<'a> SubWorld<'a> {
    /// Constructs a new SubWorld.
    ///
    /// # Safety
    /// Queries assume that this type has been constructed correctly. Ensure that sub-worlds represent
    /// disjoint portions of a world and that the world is not used while any of its sub-worlds are alive.
    pub unsafe fn new_unchecked(
        world: &'a World,
        access: &'a Access<ComponentTypeId>,
        archetypes: &'a ArchetypeAccess,
    ) -> Self {
        SubWorld {
            world,
            components: ComponentAccess::Allow(Cow::Borrowed(access)),
            archetypes: if let ArchetypeAccess::Some(ref bitset) = archetypes {
                Some(bitset)
            } else {
                None
            },
        }
    }

    /// Splits the world into two. The left world allows access only to the data declared by the view;
    /// the right world allows access to all else.
    pub fn split<'b, T: for<'v> View<'v>>(&'b mut self) -> (SubWorld<'b>, SubWorld<'b>)
    where
        'a: 'b,
    {
        let access = Access {
            reads: T::read_types(),
            writes: T::write_types(),
        };
        let (left, right) = self.components.split(access);

        (
            SubWorld {
                world: self.world,
                components: left,
                archetypes: self.archetypes,
            },
            SubWorld {
                world: self.world,
                components: right,
                archetypes: self.archetypes,
            },
        )
    }

    /// Splits the world into two. The left world allows access only to the data declared by the query's view;
    /// the right world allows access to all else.
    pub fn split_for_query<'q, V: for<'v> View<'v>, F: EntityFilter>(
        &mut self,
        _: &'q Query<V, F>,
    ) -> (SubWorld, SubWorld) {
        self.split::<V>()
    }

    fn validate_archetype_access(&self, entity: Entity) -> bool {
        if let Some(archetypes) = self.archetypes {
            if let Some(location) = (*self.world).get_entity_location(entity) {
                return (*archetypes).contains(*location.archetype());
            }
        }

        true
    }

    fn validate_reads<T: Component>(&self, entity: Entity) {
        let valid = match &self.components {
            ComponentAccess::All => true,
            ComponentAccess::Allow(restrictions) => {
                restrictions.reads.contains(&ComponentTypeId::of::<T>())
            }
            ComponentAccess::Disallow(restrictions) => {
                !restrictions.reads.contains(&ComponentTypeId::of::<T>())
            }
        };

        if !valid || !self.validate_archetype_access(entity) {
            panic!("Attempted to read a component that this system does not have declared access to. \
                Consider adding a query which contains `{}` and this entity in its result set to the system, \
                or use `SystemBuilder::read_component` to declare global access.",
                std::any::type_name::<T>());
        }
    }

    fn validate_writes<T: Component>(&self, entity: Entity) {
        let valid = match &self.components {
            ComponentAccess::All => true,
            ComponentAccess::Allow(restrictions) => {
                restrictions.writes.contains(&ComponentTypeId::of::<T>())
            }
            ComponentAccess::Disallow(restrictions) => {
                !restrictions.writes.contains(&ComponentTypeId::of::<T>())
            }
        };

        if !valid || !self.validate_archetype_access(entity) {
            panic!("Attempted to write to a component that this system does not have declared access to. \
                Consider adding a query which contains `{}` and this entity in its result set to the system, \
                or use `SystemBuilder::write_component` to declare global access.",
                std::any::type_name::<T>());
        }
    }
}

impl<'a> EntityStore for SubWorld<'a> {
    #[inline]
    fn get_component<T: Component>(&self, entity: Entity) -> Option<Ref<T>> {
        self.validate_reads::<T>(entity);
        self.world.get_component::<T>(entity)
    }

    #[inline]
    unsafe fn get_component_mut_unchecked<T: Component>(
        &self,
        entity: Entity,
    ) -> Option<RefMut<T>> {
        self.validate_writes::<T>(entity);
        self.world.get_component_mut_unchecked::<T>(entity)
    }

    #[inline]
    fn get_tag<T: Tag>(&self, entity: Entity) -> Option<&T> { self.world.get_tag(entity) }

    #[inline]
    fn is_alive(&self, entity: Entity) -> bool { self.world.is_alive(entity) }

    fn get_component_storage<V: for<'b> View<'b>>(
        &self,
    ) -> Result<StorageAccessor, ComponentAccessError> {
        if V::validate_access(&self.components) {
            Ok(StorageAccessor {
                storage: self.world.storage(),
                archetypes: self.archetypes,
            })
        } else {
            Err(ComponentAccessError)
        }
    }
}

impl<'a> From<&'a mut World> for SubWorld<'a> {
    fn from(world: &'a mut World) -> Self {
        Self {
            world,
            components: ComponentAccess::All,
            archetypes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[test]
    fn writeread_left_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (left, _) = world.split::<Write<usize>>();
        assert!(left.get_component::<usize>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn writeread_left_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (left, _) = world.split::<Write<usize>>();
        let _ = left.get_component::<bool>(entity);
    }

    #[test]
    fn writeread_right_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, right) = world.split::<Write<usize>>();
        assert!(right.get_component::<bool>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn writeread_right_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, right) = world.split::<Write<usize>>();
        let _ = right.get_component::<usize>(entity);
    }

    // --------

    #[test]
    fn readread_left_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (left, _) = world.split::<Read<usize>>();
        assert!(left.get_component::<usize>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn readread_left_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (left, _) = world.split::<Read<usize>>();
        let _ = left.get_component::<bool>(entity);
    }

    #[test]
    fn readread_right_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, right) = world.split::<Read<usize>>();
        assert!(right.get_component::<bool>(entity).is_some());
    }

    #[test]
    fn readread_right_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, right) = world.split::<Read<usize>>();
        assert!(right.get_component::<usize>(entity).is_some());
    }

    // --------

    #[test]
    fn writewrite_left_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (mut left, _) = world.split::<Write<usize>>();
        assert!(left.get_component_mut::<usize>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn writewrite_left_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (mut left, _) = world.split::<Write<usize>>();
        let _ = left.get_component_mut::<bool>(entity);
    }

    #[test]
    fn writewrite_right_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, mut right) = world.split::<Write<usize>>();
        assert!(right.get_component_mut::<bool>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn writewrite_right_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, mut right) = world.split::<Write<usize>>();
        let _ = right.get_component_mut::<usize>(entity);
    }

    // --------

    #[test]
    #[should_panic]
    fn readwrite_left_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (mut left, _) = world.split::<Read<usize>>();
        let _ = left.get_component_mut::<usize>(entity);
    }

    #[test]
    #[should_panic]
    fn readwrite_left_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (mut left, _) = world.split::<Read<usize>>();
        let _ = left.get_component_mut::<bool>(entity);
    }

    #[test]
    fn readwrite_right_included() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, mut right) = world.split::<Read<usize>>();
        assert!(right.get_component_mut::<bool>(entity).is_some());
    }

    #[test]
    #[should_panic]
    fn readwrite_right_excluded() {
        let mut world = World::new();
        let entity = world.insert((), vec![(1usize, false)])[0];

        let (_, mut right) = world.split::<Read<usize>>();
        let _ = right.get_component_mut::<usize>(entity);
    }
}
