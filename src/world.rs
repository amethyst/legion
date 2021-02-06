//! Worlds store collections of entities. An entity is a collection of components, identified
//! by a unique [Entity] ID.
//!
//! # Creating a world
//!
//! ```
//! # use legion::*;
//! let mut world = World::default();
//! ```
//!
//! `World::new()` can be used to construct a new world with custom options.
//!
//! # Inserting entities
//!
//! Entities can be inserted via either `push` (for a single entity) or `extend` (for a collection
//! of entities with the same component types). The world will create a unique ID for each entity
//! upon insertion that you can use to refer to that entity later.
//!
//! ```
//! # use legion::*;
//! # let mut world = World::default();
//! // a component is any type that is 'static, sized, send and sync
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! struct Position {
//!     x: f32,
//!     y: f32,
//! }
//!
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! struct Velocity {
//!     dx: f32,
//!     dy: f32,
//! }
//!
//! // push a component tuple into the world to create an entity
//! let entity: Entity = world.push((Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }));
//!
//! // or extend via an IntoIterator of tuples to add many at once
//! // this is faster than individual pushes
//! let entities: &[Entity] = world.extend(vec![
//!     (Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }),
//!     (Position { x: 1.0, y: 1.0 }, Velocity { dx: 0.0, dy: 0.0 }),
//!     (Position { x: 2.0, y: 2.0 }, Velocity { dx: 0.0, dy: 0.0 }),
//! ]);
//! ```
//!
//! If your data is already organized as such, you can alternatively insert entities as a
//! strucure-of-arrays.
//!
//! ```
//! # use legion::*;
//! let mut world = World::default();
//! let _entities = world.extend(
//!     (
//!         vec![1usize, 2usize, 3usize],
//!         vec![false, true, false],
//!         vec![5.3f32, 5.3f32, 5.2f32],
//!     ).into_soa()
//! );
//! ```
//!
//! SoA inserts require all vectors to have the same length. These inserts are faster than inserting
//! via an iterator of tuples.
//!
//! # Modifying entities
//!
//! Components can be added or removed from an existing entity via the [Entry] API.
//!
//! ```
//! # use legion::*;
//! # let mut world = World::default();
//! # let entity = world.push((false, 1usize));
//! // entries return `None` if the entity does not exist
//! if let Some(mut entry) = world.entry(entity) {
//!     // add an extra component
//!     entry.add_component(12f32);
//!
//!     // remove a component
//!     entry.remove_component::<usize>();
//! }
//! ```
//!
//! Note that it is significantly faster to create an entity with its initial set of components
//! via `push` or `extend` than it is to add the components one-by-one after creating the entity.
//!
//! # Accessing components
//!
//! The fastest way to access a large number of entities' components is via [queries](crate::query).
//!
//! The entry API also allows access to an individual entity's components.
//!
//! ```
//! # use legion::*;
//! # let mut world = World::default();
//! # let entity = world.push((false, 12f32));
//! // entries return `None` if the entity does not exist
//! if let Some(mut entry) = world.entry(entity) {
//!     // access information about the entity's archetype
//!     println!("{:?} has {:?}", entity, entry.archetype().layout().component_types());
//!
//!     // access the entity's components, returns `None` if the entity does not have the component
//!     assert_eq!(entry.get_component::<f32>().unwrap(), &12f32);
//! }
//! ```
//!
//! # Events
//!
//! Notifications about archetype creation and entity insertion/removal from an archetype can be sent
//! to an [EventSender] by subscribing to the world. A layout filter specifies which archetypes the
//! subscriber is interested in.
//!
//! ```ignore
//! # use legion::*;
//! # let mut world = World::default();
//! # struct Position;
//! // subscribe to events involving entities with a `Position` with a
//! // crossbeam channel.
//! let (tx, rx) = crossbeam_channel::unbounded::<legion::world::Event>();
//! world.subscribe(tx, component::<Position>());
//! ```
//!
//! # World splitting
//!
//! World splitting allows mutable access to a world via multiple entries or queries at the same time,
//! provided that their component accesses do not conflict with one another.
//!
//! ```
//! # use legion::*;
//! # struct A;
//! # struct B;
//! # struct C;
//! let mut world = World::default();
//! let entity = world.push((A, B, C));
//! let (mut left, mut right) = world.split::<(Read<A>, Write<B>)>();
//!
//! // left only has permission to read A and read/write B.
//! let b: &mut B = left.entry_mut(entity).unwrap().get_component_mut::<B>().unwrap();
//!
//! // right can access anything _but_ writes to A and read/write to B.
//! let a: &A = right.entry_ref(entity).unwrap().get_component::<A>().unwrap();
//! let c: &C = right.entry_ref(entity).unwrap().get_component::<C>().unwrap();
//! ```

pub use crate::internals::{
    entity::{Allocate, Entity, EntityHasher, EntityLocation, LocationMap},
    entry::{ComponentError, Entry, EntryMut, EntryRef},
    event::{Event, EventSender},
    permissions::Permissions,
    subworld::{ArchetypeAccess, ComponentAccess, SubWorld},
    world::{
        Duplicate, EntityAccessError, EntityRewrite, EntityStore, Merger, StorageAccessor, World,
        WorldId, WorldOptions,
    },
};
