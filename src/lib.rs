//! Legion aims to be a feature rich high performance ECS library for Rust game projects with minimal boilerplate.
//!
//! # Benchmarks
//!
//! ![](https://raw.githubusercontent.com/TomGillen/legion/master/bench.png)
//!
//! # Getting Started
//!
//! ```rust
//! use legion::prelude::*;
//!
//! // Define our entity data types
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
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! struct Model(usize);
//!
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! struct Static;
//!
//! // Create a world to store our entities
//! let universe = Universe::new(None);
//! let mut world = universe.create_world();
//!
//! // Create entities with `Position` and `Velocity` data
//! world.insert_from(
//!     (),
//!     (0..999).map(|_| (Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }))
//! );
//!
//! // Create entities with `Position` data and a tagged with `Model` data and as `Static`
//! // Tags are shared across many entities, and enable further batch processing and filtering use cases
//! world.insert_from(
//!     (Model(5), Static).as_tags(),
//!     (0..999).map(|_| (Position { x: 0.0, y: 0.0 },))
//! );
//!
//! // Create a query which finds all `Position` and `Velocity` components
//! let mut query = <(Write<Position>, Read<Velocity>)>::query();
//!
//! // Iterate through all entities that match the query in the world
//! for (pos, vel) in query.iter(&world) {
//!     pos.x += vel.dx;
//!     pos.y += vel.dy;
//! }
//! ```
//!
//! # Features
//!
//! Legion aims to be a more complete game-ready ECS than many of its predecessors.
//!
//! ### Advanced Query Filters
//!
//! The query API can do much more than pull entity data out of the world.
//!
//! Additional data type filters:
//!
//! ```rust
//! # use legion::prelude::*;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Position {
//! #     x: f32,
//! #     y: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Velocity {
//! #     dx: f32,
//! #     dy: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Model(usize);
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Static;
//! # let universe = Universe::new(None);
//! # let mut world = universe.create_world();
//! // It is possible to specify that entities must contain data beyond that being fetched
//! let mut query = Read::<Position>::query()
//!     .filter(component::<Velocity>());
//! for position in query.iter(&world) {
//!     // these entities also have `Velocity`
//! }
//! ```
//!
//! Filter boolean operations:
//!
//! ```rust
//! # use legion::prelude::*;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Position {
//! #     x: f32,
//! #     y: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Velocity {
//! #     dx: f32,
//! #     dy: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Model(usize);
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Static;
//! # let universe = Universe::new(None);
//! # let mut world = universe.create_world();
//! // Filters can be combined with boolean operators
//! let mut query = Read::<Position>::query()
//!     .filter(tag::<Static>() | !component::<Velocity>());
//! for position in query.iter(&world) {
//!     // these entities are also either marked as `Static`, or do *not* have a `Velocity`
//! }
//! ```
//!
//! Filter by tag data value:
//!
//! ```rust
//! # use legion::prelude::*;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Position {
//! #     x: f32,
//! #     y: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Velocity {
//! #     dx: f32,
//! #     dy: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Model(usize);
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Static;
//! # let universe = Universe::new(None);
//! # let mut world = universe.create_world();
//! // Filters can filter by specific tag values
//! let mut query = Read::<Position>::query()
//!     .filter(tag_value(&Model(3)));
//! for position in query.iter(&world) {
//!     // these entities all have tag value `Model(3)`
//! }
//! ```
//!
//! Change detection:
//!
//! ```rust
//! # use legion::prelude::*;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Position {
//! #     x: f32,
//! #     y: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Velocity {
//! #     dx: f32,
//! #     dy: f32,
//! # }
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Model(usize);
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Static;
//! # let universe = Universe::new(None);
//! # let mut world = universe.create_world();
//! // Queries can perform coarse-grained change detection, rejecting entities who's data
//! // has not changed since the last time the query was iterated.
//! let mut query = <(Read<Position>, Tagged<Model>)>::query()
//!     .filter(changed::<Position>());
//! for (pos, model) in query.iter(&world) {
//!     // entities who have changed position
//! }
//! ```
//!
//! ### Content Streaming
//!
//! Entities can be loaded and initialized in a background `World` on separate threads and then
//! when ready, merged into the main `World` near instantaneously.
//!
//! ```rust
//! # use legion::prelude::*;
//! let universe = Universe::new(None);
//! let mut world_a = universe.create_world();
//! let mut world_b = universe.create_world();
//!
//! // Merge all entities from `world_b` into `world_a`
//! // Entity IDs are guarenteed to be unique across worlds and will
//! // remain unchanged across the merge.
//! world_a.merge(world_b);
//! ```
//!
//! ### Chunk Iteration
//!
//! Entity data is allocated in blocks called "chunks", each approximately containing 64KiB of data. The query API exposes each chunk via 'iter_chunk'. As all entities in a chunk are guarenteed to contain the same set of entity data and shared data values, it is possible to do batch processing via the chunk API.
//!
//! ```rust
//! # use legion::prelude::*;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Transform;
//! # #[derive(Clone, Copy, Debug, PartialEq)]
//! # struct Model(usize);
//! # let universe = Universe::new(None);
//! # let mut world = universe.create_world();
//! fn render_instanced(model: &Model, transforms: &[Transform]) {
//!     // pass `transforms` pointer to graphics API to load into constant buffer
//!     // issue instanced draw call with model data and transforms
//! }
//!
//! let mut query = Read::<Transform>::query()
//!     .filter(tag::<Model>());
//!
//! for chunk in query.iter_chunks(&world) {
//!     // get the chunk's model
//!     let model: &Model = chunk.tag().unwrap();
//!
//!     // get a (runtime borrow checked) slice of transforms
//!     let transforms = chunk.components::<Transform>().unwrap();
//!
//!     // give the model and transform slice to our renderer
//!     render_instanced(model, &transforms);
//! }
//! ```

pub mod borrows;
pub mod query;
pub mod storage;

pub mod experimental;

use crate::borrows::*;
use crate::storage::TagValue;
use crate::storage::*;
use std::fmt::Debug;
use std::marker::PhantomData;

use fnv::FnvHashSet;
use parking_lot::Mutex;
use slog::{debug, info, o, trace, Drain};
use std::any::TypeId;
use std::fmt::Display;
use std::iter::Peekable;
use std::num::Wrapping;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub mod prelude {
    pub use crate::query::{filter::*, IntoQuery, Query, Read, Tagged, Write};
    pub use crate::{Entity, IntoTagSet, Universe, World};
}

/// Unique world ID.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct WorldId(u16);

impl WorldId {
    fn archetype(&self, id: u16) -> ArchetypeId {
        ArchetypeId(self.0, id)
    }
}

/// Unique Archetype ID.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ArchetypeId(u16, u16);

impl ArchetypeId {
    fn chunk(&self, id: u16) -> ChunkId {
        ChunkId(self.0, self.1, id)
    }
}

/// Unique Chunk ID.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ChunkId(u16, u16, u16);

pub(crate) type EntityIndex = u32;
pub(crate) type EntityVersion = Wrapping<u32>;

/// A handle to an entity.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Entity {
    index: EntityIndex,
    version: EntityVersion,
}

impl Entity {
    pub(crate) fn new(index: EntityIndex, version: EntityVersion) -> Entity {
        Entity {
            index: index,
            version: version,
        }
    }
}

impl Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}#{}", self.index, self.version)
    }
}

/// The `Universe` is a factory for creating `World`s.
///
/// Entities inserted into worlds created within the same universe are guarenteed to have
/// unique `Entity` IDs, even across worlds.
///
/// Worlds belonging to the same universe can be safely merged via `World.merge`.
#[derive(Debug)]
pub struct Universe {
    name: String,
    logger: slog::Logger,
    allocator: Arc<Mutex<BlockAllocator>>,
    next_id: AtomicUsize,
}

impl Universe {
    /// Creates a new `Universe`.
    ///
    /// # Examples
    /// ```
    /// # use slog::*;
    /// # use std::sync::Mutex;
    /// # use legion::prelude::*;
    /// // Create an slog logger
    /// let decorator = slog_term::TermDecorator::new().build();
    /// let drain = Mutex::new(slog_term::FullFormat::new(decorator).build()).fuse();
    /// let log = slog::Logger::root(drain, o!());
    ///
    /// // Create world with logger
    /// let universe = Universe::new(log);
    ///
    /// // Create world without logger
    /// let universe = Universe::new(None);
    /// ```
    pub fn new<L: Into<Option<slog::Logger>>>(logger: L) -> Self {
        let name = names::Generator::default().next().unwrap();
        let logger = logger
            .into()
            .unwrap_or(slog::Logger::root(slog_stdlog::StdLog.fuse(), o!()))
            .new(o!("universe" => name.clone()));

        info!(logger, "starting universe");
        Universe {
            name,
            logger,
            allocator: Arc::from(Mutex::new(BlockAllocator::new())),
            next_id: AtomicUsize::new(0),
        }
    }

    /// Creates a new `World` within this `Universe`.
    ///
    /// Entities inserted into worlds created within the same universe are guarenteed to have
    /// unique `Entity` IDs, even across worlds.
    ///
    /// Worlds belonging to the same universe can be safely merged via `World.merge`.
    pub fn create_world(&self) -> World {
        World::new(
            WorldId(self.next_id.fetch_add(1, Ordering::SeqCst) as u16),
            self.logger.clone(),
            EntityAllocator::new(self.allocator.clone()),
        )
    }
}

pub(crate) type ComponentIndex = u16;
pub(crate) type ChunkIndex = u16;
pub(crate) type ArchetypeIndex = u16;

#[derive(Debug)]
struct BlockAllocator {
    allocated: usize,
    free: Vec<EntityBlock>,
}

impl BlockAllocator {
    const BLOCK_SIZE: usize = 1024;

    pub fn new() -> Self {
        BlockAllocator {
            allocated: 0,
            free: Vec::new(),
        }
    }

    pub fn allocate(&mut self) -> EntityBlock {
        if let Some(block) = self.free.pop() {
            block
        } else {
            let block = EntityBlock::new(self.allocated as EntityIndex, BlockAllocator::BLOCK_SIZE);
            self.allocated += BlockAllocator::BLOCK_SIZE;
            block
        }
    }

    pub fn free(&mut self, block: EntityBlock) {
        self.free.push(block);
    }
}

#[derive(Debug)]
struct EntityBlock {
    start: EntityIndex,
    len: usize,
    versions: Vec<EntityVersion>,
    free: Vec<EntityIndex>,
    locations: Vec<(ArchetypeIndex, ChunkIndex, ComponentIndex)>,
}

impl EntityBlock {
    pub fn new(start: EntityIndex, len: usize) -> EntityBlock {
        EntityBlock {
            start: start,
            len: len,
            versions: Vec::with_capacity(len),
            free: Vec::new(),
            locations: std::iter::repeat((
                0 as ArchetypeIndex,
                0 as ChunkIndex,
                0 as ComponentIndex,
            ))
            .take(len)
            .collect(),
        }
    }

    fn index(&self, index: EntityIndex) -> usize {
        (index - self.start) as usize
    }

    pub fn in_range(&self, index: EntityIndex) -> bool {
        index >= self.start && index < (self.start + self.len as u32)
    }

    pub fn is_alive(&self, entity: &Entity) -> Option<bool> {
        if entity.index >= self.start {
            let i = self.index(entity.index);
            self.versions.get(i).map(|v| *v == entity.version)
        } else {
            None
        }
    }

    pub fn allocate(&mut self) -> Option<Entity> {
        if let Some(index) = self.free.pop() {
            let i = self.index(index);
            Some(Entity::new(index, self.versions[i]))
        } else if self.versions.len() < self.len {
            let index = self.start + self.versions.len() as EntityIndex;
            self.versions.push(Wrapping(1));
            Some(Entity::new(index, Wrapping(1)))
        } else {
            None
        }
    }

    pub fn free(&mut self, entity: Entity) -> Option<bool> {
        if let Some(alive) = self.is_alive(&entity) {
            let i = self.index(entity.index);
            self.versions[i] += Wrapping(1);
            self.free.push(entity.index);
            Some(alive)
        } else {
            None
        }
    }

    pub fn set_location(
        &mut self,
        entity: &EntityIndex,
        location: (ArchetypeIndex, ChunkIndex, ComponentIndex),
    ) {
        assert!(*entity >= self.start);
        let index = (entity - self.start) as usize;
        *self.locations.get_mut(index).unwrap() = location;
    }

    pub fn get_location(
        &self,
        entity: &EntityIndex,
    ) -> Option<(ArchetypeIndex, ChunkIndex, ComponentIndex)> {
        if *entity < self.start {
            return None;
        }

        let index = (entity - self.start) as usize;
        self.locations.get(index).map(|x| *x)
    }
}

/// Manages the allocation and deletion of `Entity` IDs within a world.
#[derive(Debug)]
pub struct EntityAllocator {
    allocator: Arc<Mutex<BlockAllocator>>,
    blocks: Vec<EntityBlock>,
    entity_buffer: Vec<Entity>,
}

impl EntityAllocator {
    fn new(allocator: Arc<Mutex<BlockAllocator>>) -> Self {
        EntityAllocator {
            allocator: allocator,
            blocks: Vec::new(),
            entity_buffer: Vec::new(),
        }
    }

    /// Determines if the given `Entity` is considered alive.
    pub fn is_alive(&self, entity: &Entity) -> bool {
        self.blocks
            .iter()
            .filter_map(|b| b.is_alive(entity))
            .nth(0)
            .unwrap_or(false)
    }

    /// Allocates a new unused `Entity` ID.
    pub fn create_entity(&mut self) -> Entity {
        let entity = if let Some(entity) = self
            .blocks
            .iter_mut()
            .rev()
            .filter_map(|b| b.allocate())
            .nth(0)
        {
            entity
        } else {
            let mut block = self.allocator.lock().allocate();
            let entity = block.allocate().unwrap();
            self.blocks.push(block);
            entity
        };

        self.entity_buffer.push(entity.clone());
        entity
    }

    pub(crate) fn delete_entity(&mut self, entity: Entity) -> bool {
        self.blocks
            .iter_mut()
            .filter_map(|b| b.free(entity))
            .nth(0)
            .unwrap_or(false)
    }

    pub(crate) fn set_location(
        &mut self,
        entity: &EntityIndex,
        location: (ArchetypeIndex, ChunkIndex, ComponentIndex),
    ) {
        self.blocks
            .iter_mut()
            .rev()
            .filter(|b| b.in_range(*entity))
            .next()
            .unwrap()
            .set_location(entity, location);
    }

    pub(crate) fn get_location(
        &self,
        entity: &EntityIndex,
    ) -> Option<(ArchetypeIndex, ChunkIndex, ComponentIndex)> {
        self.blocks
            .iter()
            .filter(|b| b.in_range(*entity))
            .next()
            .and_then(|b| b.get_location(entity))
    }

    pub(crate) fn allocation_buffer(&self) -> &[Entity] {
        self.entity_buffer.as_slice()
    }

    pub(crate) fn clear_allocation_buffer(&mut self) {
        self.entity_buffer.clear();
    }

    pub(crate) fn merge(&mut self, mut other: EntityAllocator) {
        assert!(Arc::ptr_eq(&self.allocator, &other.allocator));
        self.blocks.append(&mut other.blocks);
    }
}

impl Drop for EntityAllocator {
    fn drop(&mut self) {
        for block in self.blocks.drain(..) {
            self.allocator.lock().free(block);
        }
    }
}

/// Contains queryable collections of data associated with `Entity`s.
pub struct World {
    id: WorldId,
    logger: slog::Logger,
    allocator: EntityAllocator,
    archetypes: Vec<Archetype>,
    next_arch_id: u16,
}

impl World {
    fn new(id: WorldId, logger: slog::Logger, allocator: EntityAllocator) -> Self {
        let logger = logger.new(o!("world_id" => id.0));

        info!(logger, "starting world");
        World {
            id,
            logger,
            allocator: allocator,
            archetypes: Vec::new(),
            next_arch_id: 0,
        }
    }

    /// Merges two worlds together.
    ///
    /// This function moves all chunks from `other` into `self`. This operation is very fast,
    /// however the resulting memory layout may be inefficient if `other` contains a very small
    /// number of entities.
    ///
    /// Merge is most effectively used to allow large numbers of entities to be loaded and
    /// initialized in the background, and then shunted into the "main" world all at once, once ready.
    ///
    /// # Safety
    ///
    /// It is only safe to merge worlds which belong to the same `Universe`. This is currently not
    /// validated by the API.
    pub fn merge(&mut self, mut other: World) {
        self.allocator.merge(other.allocator);

        let first_new_index = self.archetypes.len();
        self.archetypes.append(&mut other.archetypes);

        for archetype_index in first_new_index..self.archetypes.len() {
            let archetype = self.archetypes.get(archetype_index).unwrap();
            for (chunk_index, chunk) in archetype.chunks().iter().enumerate() {
                for (entity_index, entity) in unsafe { chunk.entities().iter().enumerate() } {
                    self.allocator.set_location(
                        &entity.index,
                        (
                            archetype_index as ArchetypeIndex,
                            chunk_index as ChunkIndex,
                            entity_index as ComponentIndex,
                        ),
                    );
                }
            }
        }
    }

    /// Determines if the given `Entity` is alive within this `World`.
    pub fn is_alive(&self, entity: &Entity) -> bool {
        self.allocator.is_alive(entity)
    }

    /// Inserts entities from an iterator of component tuples.
    ///
    /// # Examples
    ///
    /// Inserting entity tuples:
    ///
    /// ```
    /// # use legion::prelude::*;
    /// # #[derive(Copy, Clone, Debug, PartialEq)]
    /// # struct Position(f32);
    /// # #[derive(Copy, Clone, Debug, PartialEq)]
    /// # struct Rotation(f32);
    /// # let universe = Universe::new(None);
    /// # let mut world = universe.create_world();
    /// # let model = 0u8;
    /// # let color = 0u16;
    /// let tags = (model, color).as_tags();
    /// let data = vec![
    ///     (Position(0.0), Rotation(0.0)),
    ///     (Position(1.0), Rotation(1.0)),
    ///     (Position(2.0), Rotation(2.0)),
    /// ];
    /// world.insert_from(tags, data);
    /// ```
    pub fn insert_from<T, C>(&mut self, tags: T, components: C) -> &[Entity]
    where
        T: TagSet,
        C: IntoIterator,
        C::Item: ComponentSet,
        IterEntitySource<C::IntoIter, C::Item>: EntitySource,
    {
        let source = C::Item::component_source(components.into_iter());
        self.insert(tags, source)
    }

    /// Inserts entities from an `EntitySource`.
    pub fn insert<T, C>(&mut self, tags: T, mut components: C) -> &[Entity]
    where
        T: TagSet,
        C: EntitySource,
    {
        // find or create archetype
        let (arch_index, archetype) = World::prep_archetype(
            &self.id,
            &mut self.archetypes,
            &mut self.next_arch_id,
            &mut self.logger,
            &tags,
            &components,
        );

        self.allocator.clear_allocation_buffer();

        // insert components into chunks
        while !components.is_empty() {
            // find or create chunk
            let (chunk_index, chunk) = archetype.get_or_create_chunk(&tags, &components);

            // insert as many components as we can into the chunk
            let allocated = components.write(chunk, &mut self.allocator);
            chunk.validate();

            // record new entity locations
            let start = unsafe { chunk.entities().len() - allocated };
            let added = unsafe { chunk.entities().iter().enumerate().skip(start) };
            for (i, e) in added {
                let comp_id = i as ComponentIndex;
                self.allocator
                    .set_location(&e.index, (arch_index, chunk_index, comp_id));
            }

            trace!(
                self.logger,
                "appended {entity_count} entities into chunk",
                entity_count = allocated;
                "archetype_id" => arch_index,
                "chunk_id" => chunk_index
            );
        }

        trace!(
            self.logger,
            "inserted {entity_count_added} entities",
            entity_count_added = self.allocator.allocation_buffer().len();
            "archetype_id" => arch_index
        );

        self.allocator.allocation_buffer()
    }

    /// Removes the given `Entity` from the `World`.
    ///
    /// Returns `true` if the entity was deleted; else `false`.
    pub fn delete(&mut self, entity: Entity) -> bool {
        let deleted = self.allocator.delete_entity(entity);

        if deleted {
            // lookup entity location
            let ids = self.allocator.get_location(&entity.index);

            // swap remove with last entity in chunk
            let swapped = ids.and_then(|(archetype_id, chunk_id, component_id)| {
                self.archetypes
                    .get_mut(archetype_id as usize)
                    .and_then(|archetype| archetype.chunk_mut(chunk_id))
                    .and_then(|chunk| chunk.remove(component_id))
            });

            // record swapped entity's new location
            if let Some(swapped) = swapped {
                self.allocator.set_location(&swapped.index, ids.unwrap());
            }
        }

        deleted
    }

    /// Mutates the composition of an entity in-place. This allows components and tags to be added
    /// or removed from an entity.
    ///
    /// # Performance
    ///
    /// Mutating an entity is *significantly slower* than inserting a new entity. Always prefer to
    /// create entities with the desired layout in the first place, and avoid adding or removing
    /// components or tags from existing entities.
    ///
    /// # Examples
    ///
    /// ```
    /// # use legion::prelude::*;
    /// # use std::sync::Arc;
    /// # #[derive(Copy, Clone, Debug, PartialEq)]
    /// # struct Static;
    /// # #[derive(Copy, Clone, Debug, PartialEq)]
    /// # struct Position(f32);
    /// # #[derive(Copy, Clone, Debug, PartialEq)]
    /// # struct Rotation(f32);
    /// # let universe = Universe::new(None);
    /// # let mut world = universe.create_world();
    /// # let model = 0u8;
    /// # let color = 0u16;
    /// # let tags = (model, color).as_tags();
    /// # let data = vec![
    /// #     (Position(0.0), Rotation(0.0)),
    /// #     (Position(1.0), Rotation(1.0)),
    /// #     (Position(2.0), Rotation(2.0)),
    /// # ];
    /// # let entity = *world.insert_from(tags, data).get(0).unwrap();
    /// world.mutate_entity(entity, |e| {
    ///     e.set_tag(Arc::new(Static));
    ///     e.add_component(Position(4.0));
    ///     e.remove_component::<Rotation>();
    /// });
    /// ```
    pub fn mutate_entity<'env, F: FnOnce(&mut MutEntity<'env>)>(&mut self, entity: Entity, f: F) {
        assert!(self.is_alive(&entity));

        if let Some((arch_id, chunk_id, comp_id)) = self.allocator.get_location(&entity.index) {
            if let Some((swapped, tags, components)) = self
                .archetypes
                .get_mut(arch_id as usize)
                .and_then(|a| a.chunk_mut(chunk_id))
                .map(|c| c.fetch_remove(comp_id))
            {
                let mut mut_handle = MutEntity::<'env> {
                    tags,
                    components,
                    _phantom: PhantomData,
                };

                // mutate the entity
                f(&mut mut_handle);

                // record swapped entity's new location
                if let Some(swapped) = swapped {
                    self.allocator
                        .set_location(&swapped.index, (arch_id, chunk_id, comp_id));
                }

                // re-insert the entity
                self.insert(mut_handle.tags, mut_handle.components);
            }
        }
    }

    /// Borrows entity data for the given entity.
    ///
    /// Returns `Some(data)` if the entity was found and contains the specified data.
    /// Otherwise `None` is returned.
    ///
    /// # Panics
    ///
    /// This function borrows all components of type `T` in the world. It may panic if
    /// any other code is currently borrowing `T` mutably (such as in a query).
    pub fn component<'a, T: Component>(&'a self, entity: Entity) -> Option<Borrowed<'a, T>> {
        self.allocator.get_location(&entity.index).and_then(
            |(archetype_id, chunk_id, component_id)| {
                self.archetypes
                    .get(archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(chunk_id))
                    .and_then(|chunk| chunk.components::<T>())
                    .and_then(|vec| vec.single(component_id as usize))
            },
        )
    }

    /// Mutably borrows entity data for the given entity.
    ///
    /// Returns `Some(data)` if the entity was found and contains the specified data.
    /// Otherwise `None` is returned.
    pub fn component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let archetypes = &self.archetypes;
        self.allocator.get_location(&entity.index).and_then(
            |(archetype_id, chunk_id, component_id)| {
                archetypes
                    .get(archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(chunk_id))
                    .and_then(|chunk| unsafe { chunk.components_mut_unchecked::<T>() })
                    .and_then(|vec| vec.get_mut(component_id as usize))
            },
        )
    }

    /// Borrows tag data for the given entity.
    ///
    /// Returns `Some(data)` if the entity was found and contains the specified data.
    /// Otherwise `None` is returned.
    pub fn tag<T: Tag>(&self, entity: Entity) -> Option<&T> {
        self.allocator
            .get_location(&entity.index)
            .and_then(|(archetype_id, chunk_id, _)| {
                self.archetypes
                    .get(archetype_id as usize)
                    .and_then(|archetype| archetype.chunk(chunk_id))
                    .and_then(|chunk| chunk.tag::<T>())
            })
    }

    fn prep_archetype<'a, T: TagSet, C: EntitySource>(
        id: &WorldId,
        archetypes: &'a mut Vec<Archetype>,
        next_arch_id: &mut u16,
        logger: &slog::Logger,
        tags: &T,
        components: &C,
    ) -> (ArchetypeIndex, &'a mut Archetype) {
        match archetypes
            .iter()
            .enumerate()
            .filter(|(_, a)| components.is_archetype_match(a) && tags.is_archetype_match(a))
            .map(|(i, _)| i)
            .next()
        {
            Some(i) => (i as ArchetypeIndex, unsafe {
                archetypes.get_unchecked_mut(i)
            }),
            None => {
                let archetype_id = id.archetype(*next_arch_id);
                let logger = logger.new(o!("archetype_id" => archetype_id.1));
                *next_arch_id += 1;

                let archetype = Archetype::new(
                    archetype_id,
                    logger.clone(),
                    components.types(),
                    tags.types(),
                );
                archetypes.push(archetype);

                debug!(logger, "allocated archetype");

                (
                    (archetypes.len() - 1) as ArchetypeIndex,
                    archetypes.last_mut().unwrap(),
                )
            }
        }
    }
}

pub struct MutEntity<'env> {
    tags: DynamicTagSet,
    components: DynamicSingleEntitySource,
    _phantom: PhantomData<&'env mut &'env ()>,
}

impl<'env> MutEntity<'env> {
    pub fn deconstruct(&mut self) -> (&mut DynamicTagSet, &mut DynamicSingleEntitySource) {
        (&mut self.tags, &mut self.components)
    }

    pub fn set_tag<T: Tag>(&mut self, tag: Arc<T>) {
        self.tags.set_tag(tag);
    }

    pub fn remove_tag<T: Tag>(&mut self) -> bool {
        self.tags.remove_tag::<T>()
    }

    pub fn add_component<T: Component>(&mut self, component: T) {
        self.components.add_component(component);
    }

    pub fn remove_component<T: Component>(&mut self) -> bool {
        self.components.remove_component::<T>()
    }
}

/// Inserts tags into a `Chunk` in a `World`.
pub trait TagSet {
    /// Determines if the given archetype is compatible with the data
    /// contained in the data set.
    fn is_archetype_match(&self, archetype: &Archetype) -> bool;

    /// Determines if the given chunk is compatible with the data
    /// contained in the data set.
    fn is_chunk_match(&self, chunk: &Chunk) -> bool;

    /// Configures a new chunk to include the tags in this data set.
    fn configure_chunk(&self, chunk: &mut ChunkBuilder);

    /// Gets the type of tags contained in this data set.
    fn types(&self) -> FnvHashSet<TypeId>;
}

/// A set of entity data components.
pub trait ComponentSet: Sized {
    /// Converts an iterator of `Self` into an `EntitySource`.
    fn component_source<T>(source: T) -> IterEntitySource<T, Self>
    where
        T: Iterator<Item = Self>;
}

/// Inserts entity data into a `Chunk` in a `World`.
pub trait EntitySource {
    /// Determines if the given archetype is compatible with the data
    /// contained in the source.
    fn is_archetype_match(&self, archetype: &Archetype) -> bool;

    /// Configures a new chunk to support the data contained within this source.
    fn configure_chunk(&self, chunk: &mut ChunkBuilder);

    /// Gets the entity data component types contained within this source.
    fn types(&self) -> FnvHashSet<TypeId>;

    /// Determines if the source is empty.
    fn is_empty(&mut self) -> bool;

    /// Writes as many entities into the given `Chunk` as possible, consuming the
    /// data in `self`.
    ///
    /// The provided `EntityAllocator` can be used to allocate new `Entity` IDs.
    ///
    /// Returns the number of entities written.
    fn write<'a>(&mut self, chunk: &'a mut Chunk, allocator: &mut EntityAllocator) -> usize;
}

impl TagSet for () {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool {
        archetype.tags.len() == 0
    }

    fn is_chunk_match(&self, _: &Chunk) -> bool {
        true
    }

    fn configure_chunk(&self, _: &mut ChunkBuilder) {}

    fn types(&self) -> FnvHashSet<TypeId> {
        FnvHashSet::default()
    }
}

pub trait IntoTagSet<T: TagSet> {
    fn as_tags(self) -> T;
}

macro_rules! impl_shared_data_set {
    ( $arity: expr; $( $ty: ident ),* ) => {
        impl<$( $ty ),*> IntoTagSet<($( Arc<$ty>, )*)> for ($( $ty, )*)
        where $( $ty: Tag ),*
        {
            fn as_tags(self) -> ($( Arc<$ty>, )*) {
                #![allow(non_snake_case)]
                let ($($ty,)*) = self;
                (
                    $( Arc::new($ty), )*
                )
            }
        }

        impl<$( $ty ),*> TagSet for ($( Arc<$ty>, )*)
        where $( $ty: Tag ),*
        {
            fn is_archetype_match(&self, archetype: &Archetype) -> bool {
                archetype.tags.len() == $arity &&
                $( archetype.tags.contains(&TypeId::of::<$ty>()) )&&*
            }

            fn is_chunk_match(&self, chunk: &Chunk) -> bool {
                #![allow(non_snake_case)]
                let ($($ty,)*) = self;
                $(
                    (chunk.tag::<$ty>().unwrap() == $ty.as_ref())
                )&&*
            }

            fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
                #![allow(non_snake_case)]
                let ($( ref $ty, )*) = self;
                $( chunk.register_tag($ty.clone()); )*
            }

            fn types(&self) -> FnvHashSet<TypeId> {
                [$( TypeId::of::<$ty>() ),*].iter().cloned().collect()
            }
        }
    }
}

impl_shared_data_set!(1; A);
impl_shared_data_set!(2; A, B);
impl_shared_data_set!(3; A, B, C);
impl_shared_data_set!(4; A, B, C, D);
impl_shared_data_set!(5; A, B, C, D, E);

#[doc(hidden)]
pub struct IterEntitySource<T: Iterator<Item = K>, K> {
    source: Peekable<T>,
}

macro_rules! impl_component_source {
    ( $arity: expr; $( $ty: ident => $id: ident ),* ) => {
        impl<$( $ty ),*> ComponentSet for ($( $ty, )*)
        where $( $ty: Component ),*
        {
            fn component_source<T>(source: T) -> IterEntitySource<T, Self>
                where T: Iterator<Item=Self>
            {
                IterEntitySource::<T, Self> { source: source.peekable() }
            }
        }

        impl<I, $( $ty ),*> EntitySource for IterEntitySource<I, ($( $ty, )*)>
        where I: Iterator<Item=($( $ty, )*)>,
              $( $ty: Component ),*
        {
            fn types(&self) -> FnvHashSet<TypeId> {
                [$( TypeId::of::<$ty>() ),*].iter().cloned().collect()
            }

            fn is_archetype_match(&self, archetype: &Archetype) -> bool {
                archetype.components.len() == $arity &&
                $(
                    archetype.components.contains(&TypeId::of::<$ty>())
                )&&*
            }

            fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
                $(
                    chunk.register_component::<$ty>();
                )*
            }

            fn is_empty(&mut self) -> bool {
                self.source.peek().is_none()
            }

            fn write<'a>(&mut self, chunk: &'a mut Chunk, allocator: &mut EntityAllocator) -> usize {
                #![allow(non_snake_case)]
                let mut count = 0;

                unsafe {
                    let entities = chunk.entities_unchecked();
                    $(
                        let $ty = chunk.components_mut_unchecked::<$ty>().unwrap();
                    )*

                    while let Some(($( $id, )*)) = { if chunk.is_full() { None } else { self.source.next() } } {
                        let entity = allocator.create_entity();
                        entities.push(entity);
                        $(
                            $ty.push($id);
                        )*
                        count += 1;
                    }
                }

                count
            }
        }
    }
}

impl_component_source!(1; A => a);
impl_component_source!(2; A => a, B => b);
impl_component_source!(3; A => a, B => b, C => c);
impl_component_source!(4; A => a, B => b, C => c, D => d);
impl_component_source!(5; A => a, B => b, C => c, D => d, E => e);

/// Components that are stored once per entity.
pub trait Component: Send + Sync + Sized + Debug + 'static {}

/// Components that are shared across multiple entities.
pub trait Tag: Send + Sync + Sized + PartialEq + TagValue + Debug + 'static {}

impl<T: Send + Sync + Sized + Debug + 'static> Component for T {}

impl<T: Send + Sized + PartialEq + Sync + TagValue + Debug + 'static> Tag for T {}

impl<T: Send + Sized + PartialEq + Sync + Debug + 'static> TagValue for T {
    fn downcast_equals(&self, other: &TagValue) -> bool {
        if let Some(other) = other.downcast_ref::<Self>() {
            other == self
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn create_universe() {
        Universe::new(None);
    }

    #[test]
    fn create_world() {
        let universe = Universe::new(None);
        universe.create_world();
    }

    #[test]
    fn create_entity() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        allocator.create_entity();
    }

    #[test]
    fn create_entity_many() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..512 {
            allocator.create_entity();
        }
    }

    #[test]
    fn create_entity_many_blocks() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..3000 {
            allocator.create_entity();
        }
    }

    #[test]
    fn create_entity_recreate() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));

        for _ in 0..3 {
            let entities: Vec<Entity> = (0..512).map(|_| allocator.create_entity()).collect();
            for e in entities {
                allocator.delete_entity(e);
            }
        }
    }

    #[test]
    fn is_alive_allocated() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();

        assert_eq!(true, allocator.is_alive(&entity));
    }

    #[test]
    fn is_alive_unallocated() {
        let allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = Entity::new(10 as EntityIndex, Wrapping(10));

        assert_eq!(false, allocator.is_alive(&entity));
    }

    #[test]
    fn is_alive_killed() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();
        allocator.delete_entity(entity);

        assert_eq!(false, allocator.is_alive(&entity));
    }

    #[test]
    fn delete_entity_was_alive() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();

        assert_eq!(true, allocator.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_dead() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = allocator.create_entity();
        allocator.delete_entity(entity);

        assert_eq!(false, allocator.delete_entity(entity));
    }

    #[test]
    fn delete_entity_was_unallocated() {
        let mut allocator = EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new())));
        let entity = Entity::new(10 as EntityIndex, Wrapping(10));

        assert_eq!(false, allocator.delete_entity(entity));
    }

    #[test]
    fn multiple_allocators_unique_ids() {
        let blocks = Arc::from(Mutex::new(BlockAllocator::new()));
        let mut allocator_a = EntityAllocator::new(blocks.clone());
        let mut allocator_b = EntityAllocator::new(blocks.clone());

        let mut entities_a = FnvHashSet::<Entity>::default();
        let mut entities_b = FnvHashSet::<Entity>::default();

        for _ in 0..5 {
            entities_a.extend((0..1500).map(|_| allocator_a.create_entity()));
            entities_b.extend((0..1500).map(|_| allocator_b.create_entity()));
        }

        assert_eq!(true, entities_a.is_disjoint(&entities_b));

        for e in entities_a {
            assert_eq!(true, allocator_a.is_alive(&e));
            assert_eq!(false, allocator_b.is_alive(&e));
        }

        for e in entities_b {
            assert_eq!(false, allocator_a.is_alive(&e));
            assert_eq!(true, allocator_b.is_alive(&e));
        }
    }

    #[test]
    fn get_component_empty_world() {
        let universe = Universe::new(None);
        let world = universe.create_world();

        assert_eq!(None, world.component::<i32>(Entity::new(0, Wrapping(0))));
    }

    #[test]
    fn get_shared_empty_world() {
        let universe = Universe::new(None);
        let world = universe.create_world();

        assert_eq!(None, world.tag::<i32>(Entity::new(0, Wrapping(0))));
    }
}
