use crate::{
    entity::Entity,
    query::{filter::EntityFilter, view::View, Query},
    storage::{archetype::ArchetypeIndex, component::ComponentTypeId},
    world::{ComponentAccessError, EntityStore, StorageAccessor, World},
};
use bit_set::BitSet;
use smallvec::SmallVec;
use std::borrow::Cow;

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

/// Describes access rights to items.
#[derive(Debug, Clone)]
pub struct Access<T> {
    pub reads: SmallVec<[T; 4]>,
    pub writes: SmallVec<[T; 4]>,
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

        let left = Self::Allow(Cow::Owned(access.clone()));
        let right = match self {
            Self::All => Self::Disallow(Cow::Owned(invert(access))),
            Self::Allow(current) => {
                if access.reads.iter().any(|t| !current.reads.contains(t))
                    || access.writes.iter().any(|t| !current.writes.contains(t))
                {
                    panic!("view accesses components unavailable in this world");
                }

                Self::Allow(Cow::Owned(subtract(current, access)))
            }
            Self::Disallow(current) => {
                if access.reads.iter().any(|t| current.reads.contains(t))
                    || access.writes.iter().any(|t| current.writes.contains(t))
                {
                    panic!("view accesses components unavailable in this world");
                }

                Self::Disallow(Cow::Owned(union(current, invert(access))))
            }
        };

        (left, right)
    }
}

/// Provides access to a subset of the entities of a `World`.
#[derive(Clone)]
pub struct SubWorld<'a> {
    world: &'a World,
    components: ComponentAccess<'a>,
    archetypes: Option<&'a BitSet>,
}

impl<'a> SubWorld<'a> {
    /// Constructs a new SubWorld.
    ///
    /// # Safety
    /// Queries assume that this type has been constructed correctly. Ensure that sub-worlds represent
    /// disjoint portions of a world and that the world is not used while any of its sub-worlds are alive.
    pub unsafe fn new_unchecked(
        world: &'a World,
        components: ComponentAccess<'a>,
        archetypes: Option<&'a BitSet>,
    ) -> Self {
        Self {
            world,
            components,
            archetypes,
        }
    }

    /// Splits the world into two. The left world allows access only to the data declared by the view;
    /// the right world allows access to all else.
    pub fn split<'b, T: for<'v> View<'v>>(&'b mut self) -> (SubWorld<'b>, SubWorld<'b>)
    where
        'a: 'b,
    {
        let access = Access {
            reads: SmallVec::from_slice(T::reads_types().as_ref()),
            writes: SmallVec::from_slice(T::writes_types().as_ref()),
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

    fn validate_archetype_access(&self, ArchetypeIndex(arch_index): ArchetypeIndex) -> bool {
        if let Some(archetypes) = self.archetypes {
            archetypes.contains(arch_index as usize)
        } else {
            true
        }
    }
}

impl<'a> EntityStore for SubWorld<'a> {
    fn get_component_storage<V: for<'b> View<'b>>(
        &self,
    ) -> Result<StorageAccessor, ComponentAccessError> {
        if V::validate_access(&self.components) {
            Ok(self
                .world
                .get_component_storage::<V>()
                .unwrap()
                .with_allowed_archetypes(self.archetypes))
        } else {
            Err(ComponentAccessError)
        }
    }

    fn entry_ref(&self, entity: Entity) -> Option<crate::entry::EntryRef> {
        if let Some(entry) = self.world.entry_ref(entity) {
            if !self.validate_archetype_access(entry.location().archetype()) {
                panic!("attempted to access an entity which is outside of the subworld");
            }
            Some(crate::entry::EntryRef {
                allowed_components: self.components.clone(),
                ..entry
            })
        } else {
            None
        }
    }

    fn entry_mut(&mut self, entity: Entity) -> Option<crate::entry::EntryMut> {
        // safety: protected by &mut self and subworld access validation
        if let Some(entry) = unsafe { self.world.entry_unchecked(entity) } {
            if !self.validate_archetype_access(entry.location().archetype()) {
                panic!("attempted to access an entity which is outside of the subworld");
            }
            Some(crate::entry::EntryMut {
                allowed_components: self.components.clone(),
                ..entry
            })
        } else {
            None
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
    use crate::{
        query::view::{read::Read, write::Write},
        world::{EntityStore, World},
    };

    #[test]
    fn writeread_left_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (left, _) = world.split::<Write<usize>>();
        assert!(left
            .entry_ref(entity)
            .unwrap()
            .get_component::<usize>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn writeread_left_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (left, _) = world.split::<Write<usize>>();
        let _ = left.entry_ref(entity).unwrap().get_component::<bool>();
    }

    #[test]
    fn writeread_right_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, right) = world.split::<Write<usize>>();
        assert!(right
            .entry_ref(entity)
            .unwrap()
            .get_component::<bool>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn writeread_right_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, right) = world.split::<Write<usize>>();
        let _ = right.entry_ref(entity).unwrap().get_component::<usize>();
    }

    // --------

    #[test]
    fn readread_left_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (left, _) = world.split::<Read<usize>>();
        assert!(left
            .entry_ref(entity)
            .unwrap()
            .get_component::<usize>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn readread_left_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (left, _) = world.split::<Read<usize>>();
        let _ = left.entry_ref(entity).unwrap().get_component::<bool>();
    }

    #[test]
    fn readread_right_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, right) = world.split::<Read<usize>>();
        assert!(right
            .entry_ref(entity)
            .unwrap()
            .get_component::<bool>()
            .is_some());
    }

    #[test]
    fn readread_right_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, right) = world.split::<Read<usize>>();
        assert!(right
            .entry_ref(entity)
            .unwrap()
            .get_component::<usize>()
            .is_some());
    }

    // --------

    #[test]
    fn writewrite_left_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (mut left, _) = world.split::<Write<usize>>();
        assert!(left
            .entry_mut(entity)
            .unwrap()
            .get_component_mut::<usize>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn writewrite_left_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (mut left, _) = world.split::<Write<usize>>();
        let _ = left.entry_mut(entity).unwrap().get_component_mut::<bool>();
    }

    #[test]
    fn writewrite_right_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, mut right) = world.split::<Write<usize>>();
        assert!(right
            .entry_mut(entity)
            .unwrap()
            .get_component_mut::<bool>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn writewrite_right_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, mut right) = world.split::<Write<usize>>();
        let _ = right
            .entry_mut(entity)
            .unwrap()
            .get_component_mut::<usize>();
    }

    // --------

    #[test]
    #[should_panic]
    fn readwrite_left_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (mut left, _) = world.split::<Read<usize>>();
        let _ = left.entry_mut(entity).unwrap().get_component_mut::<usize>();
    }

    #[test]
    #[should_panic]
    fn readwrite_left_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (mut left, _) = world.split::<Read<usize>>();
        let _ = left.entry_mut(entity).unwrap().get_component_mut::<bool>();
    }

    #[test]
    fn readwrite_right_included() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, mut right) = world.split::<Read<usize>>();
        assert!(right
            .entry_mut(entity)
            .unwrap()
            .get_component_mut::<bool>()
            .is_some());
    }

    #[test]
    #[should_panic]
    fn readwrite_right_excluded() {
        let mut world = World::new();
        let entity = world.push((1usize, false));

        let (_, mut right) = world.split::<Read<usize>>();
        let _ = right
            .entry_mut(entity)
            .unwrap()
            .get_component_mut::<usize>();
    }
}
