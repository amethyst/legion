//! Contains types related to command buffers.
//!
//! Use command buffers to enqueue changes to a world from within a system.
//! For example, creating or destroying entities.
//! Command buffers are flushed at the end of the schedule, or by adding a
//! `flush_command_buffers` step to the schedule.

use crate::{
    entity::{Entity, EntityAllocator},
    insert::{ArchetypeSource, ArchetypeWriter, ComponentSource, IntoComponentSource, KnownLength},
    storage::{EntityLayout, Component},
    world::{World, WorldId},
};
use derivative::Derivative;
use smallvec::SmallVec;
use std::ops::Range;
use std::{
    collections::VecDeque,
    iter::{Fuse, FusedIterator},
    marker::PhantomData,
    sync::Arc,
};
use tracing::{span, Level};

/// This trait can be used to implement custom world writer types that can be directly
/// inserted into the command buffer, for more custom and complex world operations. This is analogous
/// to the `CommandBuffer::exec_mut` function type, but does not perform explicit any/any archetype
/// access.
pub trait WorldWritable: Send + Sync {
    /// Destructs the writer and performs the write operations on the world.
    fn write(self: Arc<Self>, world: &mut World, cmd: &CommandBuffer);
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct InsertBufferedCommand<T> {
    #[derivative(Debug = "ignore")]
    components: T,
    entities: Range<usize>,
}

impl<T> WorldWritable for InsertBufferedCommand<T>
where
    T: ComponentSource + Send + Sync,
{
    fn write(self: Arc<Self>, world: &mut World, cmd: &CommandBuffer) {
        let consumed = Arc::try_unwrap(self).unwrap();

        world.extend(PreallocComponentSource::new(
            cmd.pending_insertion[consumed.entities].iter().copied(),
            consumed.components,
        ));
    }
}

struct PreallocComponentSource<I: Iterator<Item = Entity> + FusedIterator, C: ComponentSource> {
    entities: I,
    components: C,
}

impl<I: Iterator<Item = Entity> + FusedIterator, C: ComponentSource> IntoComponentSource
    for PreallocComponentSource<I, C>
{
    type Source = Self;

    fn into(self) -> Self::Source { self }
}

impl<I: Iterator<Item = Entity>, C: ComponentSource> PreallocComponentSource<Fuse<I>, C> {
    pub fn new(entities: I, components: C) -> Self {
        Self {
            entities: entities.fuse(),
            components,
        }
    }
}

impl<I: Iterator<Item = Entity> + FusedIterator, C: ComponentSource> ArchetypeSource
    for PreallocComponentSource<I, C>
{
    type Filter = C::Filter;
    fn filter(&self) -> Self::Filter { self.components.filter() }
    fn layout(&mut self) -> EntityLayout { self.components.layout() }
}

impl<I: Iterator<Item = Entity> + FusedIterator, C: ComponentSource> ComponentSource
    for PreallocComponentSource<I, C>
{
    fn push_components<'a>(
        &mut self,
        writer: &mut ArchetypeWriter<'a>,
        mut entities: impl Iterator<Item = Entity>,
    ) {
        let iter = ConcatIter {
            a: &mut self.entities,
            b: &mut entities,
        };
        self.components.push_components(writer, iter)
    }
}

struct ConcatIter<'a, T, A: Iterator<Item = T> + FusedIterator, B: Iterator<Item = T>> {
    a: &'a mut A,
    b: &'a mut B,
}

impl<'a, T, A: Iterator<Item = T> + FusedIterator, B: Iterator<Item = T>> Iterator
    for ConcatIter<'a, T, A, B>
{
    type Item = T;

    fn next(&mut self) -> Option<T> { self.a.next().or_else(|| self.b.next()) }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct InsertCommand<T> {
    #[derivative(Debug = "ignore")]
    components: T,
}

impl<T> WorldWritable for InsertCommand<T>
where
    T: IntoComponentSource + Send + Sync,
{
    fn write(self: Arc<Self>, world: &mut World, _: &CommandBuffer) {
        let consumed = Arc::try_unwrap(self).unwrap();
        world.extend(consumed.components);
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct DeleteEntityCommand(Entity);

impl WorldWritable for DeleteEntityCommand {
    fn write(self: Arc<Self>, world: &mut World, _: &CommandBuffer) { world.remove(self.0); }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct AddComponentCommand<C> {
    #[derivative(Debug = "ignore")]
    entity: Entity,
    #[derivative(Debug = "ignore")]
    component: C,
}

impl<C> WorldWritable for AddComponentCommand<C>
where
    C: Component,
{
    fn write(self: Arc<Self>, world: &mut World, _: &CommandBuffer) {
        let consumed = Arc::try_unwrap(self).unwrap();
        world
            .entry(consumed.entity)
            .expect("entity not found")
            .add_component(consumed.component);
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct RemoveComponentCommand<C> {
    entity: Entity,
    _marker: PhantomData<C>,
}

impl<C> WorldWritable for RemoveComponentCommand<C>
where
    C: Component,
{
    fn write(self: Arc<Self>, world: &mut World, _: &CommandBuffer) {
        world
            .entry(self.entity)
            .expect("entity not found")
            .remove_component::<C>();
    }
}

#[allow(clippy::enum_variant_names)]
enum Command {
    WriteWorld(Arc<dyn WorldWritable>),
    ExecMutWorld(Arc<dyn Fn(&mut World) + Send + Sync>),
}

/// A command buffer used to queue mutable changes to the world from a system. This buffer is automatically
/// flushed and refreshed at the beginning of every frame by `Schedule`. If `Schedule` is not used,
/// then the user needs to manually flush it by performing `CommandBuffer::flush`.
///
/// # Examples
///
/// Inserting an entity using the `CommandBuffer`:
///
/// ```
/// # use legion::systems::command::*;
/// # use legion::world::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position(f32);
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Rotation(f32);
/// # let universe = Universe::new();
/// # let mut world = universe.create_world();
/// let mut command_buffer = CommandBuffer::new(&world);
/// let entity = command_buffer.push(());
///
/// command_buffer.add_component(entity, Position(123.0));
/// command_buffer.remove(entity);
///
/// command_buffer.flush(&mut world);
/// ```
pub struct CommandBuffer {
    world_id: WorldId,
    commands: VecDeque<Command>,
    entity_allocator: EntityAllocator,
    pending_insertion: SmallVec<[Entity; 64]>,
}

impl CommandBuffer {
    /// Constructs an empty command buffer.
    pub fn new(world: &World) -> Self {
        Self {
            world_id: world.id(),
            commands: Default::default(),
            pending_insertion: SmallVec::new(),
            entity_allocator: world.entity_allocator().clone(),
        }
    }

    /// Gets the ID of the world this command buffer belongs to.
    pub fn world(&self) -> WorldId { self.world_id }

    /// Flushes this command buffer, draining all stored commands and writing them to the world.
    ///
    /// Command flushes are performed in a FIFO manner, allowing for reliable, linear commands being
    /// executed in the order they were provided.
    pub fn flush(&mut self, world: &mut World) {
        let span = span!(Level::TRACE, "Draining command buffer");
        let _guard = span.enter();

        if self.world_id != world.id() {
            panic!("command buffers may only write into their parent world");
        }

        while let Some(command) = self.commands.pop_back() {
            match command {
                Command::WriteWorld(ptr) => ptr.write(world, self),
                Command::ExecMutWorld(closure) => closure(world),
            }
        }

        self.pending_insertion.clear();
    }

    /// Executes an arbitrary closure against the mutable world, allowing for queued exclusive
    /// access to the world.
    pub fn exec_mut<F>(&mut self, f: F)
    where
        F: 'static + Fn(&mut World) + Send + Sync,
    {
        self.commands.push_front(Command::ExecMutWorld(Arc::new(f)));
    }

    /// Inserts an arbitrary implementor of the `WorldWritable` trait into the command queue.
    /// This can be leveraged for creating custom `WorldWritable` trait implementors, and is used
    /// internally for the default writers.
    fn insert_writer<W>(&mut self, writer: W)
    where
        W: 'static + WorldWritable,
    {
        self.commands
            .push_front(Command::WriteWorld(Arc::new(writer)));
    }

    /// Queues the insertion of a single entity into the world.
    pub fn push<T>(&mut self, components: T) -> Entity
    where
        Option<T>: 'static + IntoComponentSource,
        <Option<T> as IntoComponentSource>::Source: KnownLength + Send + Sync,
    {
        self.extend(Some(components))[0]
    }

    /// Queues the insertion of new entities into the world.
    pub fn extend<T>(&mut self, components: T) -> &[Entity]
    where
        T: 'static + IntoComponentSource,
        <T as IntoComponentSource>::Source: KnownLength + Send + Sync,
    {
        let components = components.into();
        let start = self.pending_insertion.len();
        let count = components.len();

        self.pending_insertion.reserve(count);
        self.pending_insertion
            .extend(self.entity_allocator.iter().take(count));

        let range = start..self.pending_insertion.len();

        self.commands
            .push_front(Command::WriteWorld(Arc::new(InsertBufferedCommand {
                components,
                entities: range.clone(),
            })));

        &self.pending_insertion[range]
    }

    /// Queues the deletion of an entity in the command buffer.
    pub fn remove(&mut self, entity: Entity) { self.insert_writer(DeleteEntityCommand(entity)); }

    /// Queues the addition of a component from an entity in the command buffer.
    pub fn add_component<C: Component>(&mut self, entity: Entity, component: C) {
        self.insert_writer(AddComponentCommand { entity, component });
    }

    /// Queues the removal of a component from an entity in the command buffer.
    pub fn remove_component<C: Component>(&mut self, entity: Entity) {
        self.insert_writer(RemoveComponentCommand {
            entity,
            _marker: PhantomData::<C>::default(),
        });
    }

    /// Returns the current number of commands already queued in this `CommandBuffer` instance.
    #[inline]
    pub fn len(&self) -> usize { self.commands.len() }

    /// Returns true if this `CommandBuffer` is currently empty and contains no writers.
    #[inline]
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{view::Read, IntoQuery};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);
    #[derive(Default)]
    struct TestResource(pub i32);

    #[test]
    fn simple_write_test() {
        let mut world = World::default();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];
        let components_len = components.len();

        let mut command = CommandBuffer::new(&world);
        let _ = command.extend(components);

        // Assert writing checks
        // TODO:
        //assert_eq!(
        //    vec![ComponentTypeId::of::<Pos>(), ComponentTypeId::of::<Vel>()],
        //    command.write_components()
        //);

        command.flush(&mut world);

        let mut query = Read::<Pos>::query();

        let mut count = 0;
        for _ in query.iter(&world) {
            count += 1;
        }

        assert_eq!(components_len, count);
    }
}
