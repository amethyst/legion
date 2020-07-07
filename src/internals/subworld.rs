//! Contains types related to the [SubWorld](struct.SubWorld.html) which
//! can split a world by component type access.

use super::{
    entity::Entity,
    entry::{EntryMut, EntryRef},
    permissions::Permissions,
    query::{filter::EntityFilter, view::View, Query},
    storage::{archetype::ArchetypeIndex, component::ComponentTypeId},
    world::{ComponentAccessError, EntityStore, StorageAccessor, World, WorldId},
};
use bit_set::BitSet;
use std::borrow::Cow;

/// Describes which archetypes are available for access.
pub enum ArchetypeAccess {
    /// All archetypes.
    All,
    /// Some archetypes.
    Some(BitSet),
}

impl ArchetypeAccess {
    /// Determines if the given archetypes are disjoint from those allowed by this archetype access,
    pub fn is_disjoint(&self, other: &ArchetypeAccess) -> bool {
        match self {
            Self::All => false,
            Self::Some(mine) => match other {
                Self::All => false,
                Self::Some(theirs) => mine.is_disjoint(theirs),
            },
        }
    }

    /// Returns a bitset of allowed archetype indexes if this access is `Some`.
    pub fn bitset(&self) -> Option<&BitSet> {
        match self {
            Self::All => None,
            Self::Some(bitset) => Some(bitset),
        }
    }
}

/// Describes which components are available for access.
#[derive(Clone)]
pub enum ComponentAccess<'a> {
    /// All component types are allowed.
    All,
    /// Some component types are allowed.
    Allow(Cow<'a, Permissions<ComponentTypeId>>),
    /// Some component types are disallowed.
    Disallow(Cow<'a, Permissions<ComponentTypeId>>),
}

impl<'a> ComponentAccess<'a> {
    /// Returns `truw` if the given component is accessible for reads.
    pub fn allows_read(&self, component: ComponentTypeId) -> bool {
        match self {
            Self::All => true,
            Self::Allow(components) => components.reads().contains(&component),
            Self::Disallow(components) => !components.reads().contains(&component),
        }
    }

    /// Returns `truw` if the given component is accessible for writes.
    pub fn allows_write(&self, component: ComponentTypeId) -> bool {
        match self {
            Self::All => true,
            Self::Allow(components) => components.writes().contains(&component),
            Self::Disallow(components) => !components.writes().contains(&component),
        }
    }

    /// Splits this permission set into two; the left access only allows the permissions given, while the right
    /// allows only what is left from this set after subtracting said permissions.
    pub(crate) fn split(&mut self, access: Permissions<ComponentTypeId>) -> (Self, Self) {
        fn append_incompatible(
            denied: &mut Permissions<ComponentTypeId>,
            to_deny: &Permissions<ComponentTypeId>,
        ) {
            // reads are now denied writes
            for read in to_deny.reads() {
                denied.push_write(*read);
            }

            // writes are now entirely denied
            for write in to_deny.writes() {
                denied.push(*write);
            }
        }

        fn incompatible(
            permissions: &Permissions<ComponentTypeId>,
        ) -> Permissions<ComponentTypeId> {
            let mut denied = Permissions::new();
            // if the current permission allows reads, then everything else must deny writes
            for read in permissions.read_only() {
                denied.push_write(*read);
            }

            // if the current permission allows writes, then everything else must deny all
            for write in permissions.writes() {
                denied.push(*write);
            }

            denied
        }

        match self {
            Self::All => {
                let denied = incompatible(&access);
                (
                    Self::Allow(Cow::Owned(access)),
                    Self::Disallow(Cow::Owned(denied)),
                )
            }
            Self::Allow(allowed) => {
                if !allowed.is_superset(&access) {
                    panic!("view accesses components unavailable in this world: world allows only {}, view requires {}", allowed, access);
                }

                let mut allowed = allowed.clone();
                allowed.to_mut().subtract(&access);

                (Self::Allow(Cow::Owned(access)), Self::Allow(allowed))
            }
            Self::Disallow(denied) => {
                if !denied.is_disjoint(&access) {
                    panic!("view accesses components unavailable in this world: world disallows {}, view requires {}", denied, access);
                }

                let mut denied = denied.clone();
                append_incompatible(denied.to_mut(), &access);

                (Self::Allow(Cow::Owned(access)), Self::Disallow(denied))
            }
        }
    }
}

/// Provides access to a subset of the entities of a `World`.
///
/// To access a component mutably in a world, such as inside a [query](../query/index.html) or via an
/// [entry](../entry/index.html), you need to borrow the entire world mutably. This prevents you from
/// accessing any other data in the world at the same time.
///
/// In some cases, we can work around this by splitting the world. We can split a world around the
/// component types requested by a [view](../query/view/index.html). This will create two subworlds,
/// the left one allowing access only to the components (and mutability) declared by the view, while
/// the right subworld will allow access to everything _but_ those components.
///
/// Subworlds can be recustively further split.
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
        let permissions = T::requires_permissions();
        let (left, right) = self.components.split(permissions);

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

    fn entry_ref(&self, entity: Entity) -> Option<EntryRef> {
        if let Some(entry) = self.world.entry_ref(entity) {
            if !self.validate_archetype_access(entry.location().archetype()) {
                panic!("attempted to access an entity which is outside of the subworld");
            }
            Some(EntryRef {
                allowed_components: self.components.clone(),
                ..entry
            })
        } else {
            None
        }
    }

    fn entry_mut(&mut self, entity: Entity) -> Option<EntryMut> {
        // safety: protected by &mut self and subworld access validation
        if let Some(entry) = unsafe { self.world.entry_unchecked(entity) } {
            if !self.validate_archetype_access(entry.location().archetype()) {
                panic!("attempted to access an entity which is outside of the subworld");
            }
            Some(EntryMut {
                allowed_components: self.components.clone(),
                ..entry
            })
        } else {
            None
        }
    }

    fn id(&self) -> WorldId { self.world.id() }
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
    use crate::internals::{
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
