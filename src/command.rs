use crate::{
    borrow::AtomicRefCell,
    cons::{ConsAppend, ConsFlatten},
    entity::Entity,
    filter::{ChunksetFilterData, Filter},
    storage::{Component, ComponentTypeId, Tag, TagTypeId},
    world::{ComponentSource, ComponentTupleSet, IntoComponentSource, TagLayout, TagSet, World},
};
use derivative::Derivative;
use smallvec::SmallVec;
use std::{collections::VecDeque, iter::FromIterator, marker::PhantomData, sync::Arc};

pub trait WorldWritable {
    fn write(self: Arc<Self>, world: &mut World);

    fn write_components(&self) -> Vec<ComponentTypeId>;
    fn write_tags(&self) -> Vec<TagTypeId>;
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct InsertBufferedCommand<T, C> {
    write_components: Vec<ComponentTypeId>,
    write_tags: Vec<TagTypeId>,

    #[derivative(Debug = "ignore")]
    tags: T,
    #[derivative(Debug = "ignore")]
    components: C,

    entities: Vec<Entity>,
}
impl<T, C> WorldWritable for InsertBufferedCommand<T, C>
where
    T: TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
    C: ComponentSource,
{
    fn write(self: Arc<Self>, world: &mut World) {
        let consumed = Arc::try_unwrap(self).unwrap();

        tracing::trace!(
            "insert_buffered, ({}. {}), {:?}",
            std::any::type_name::<T>(),
            std::any::type_name::<C>(),
            consumed.entities
        );

        world.insert_buffered(&consumed.entities, consumed.tags, consumed.components);
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { self.write_components.clone() }
    fn write_tags(&self) -> Vec<TagTypeId> { self.write_tags.clone() }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct InsertCommand<T, C> {
    write_components: Vec<ComponentTypeId>,
    write_tags: Vec<TagTypeId>,

    #[derivative(Debug = "ignore")]
    tags: T,
    #[derivative(Debug = "ignore")]
    components: C,
}
impl<T, C> WorldWritable for InsertCommand<T, C>
where
    T: TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
    C: IntoComponentSource,
{
    fn write(self: Arc<Self>, world: &mut World) {
        tracing::trace!(
            "insert, ({}. {})",
            std::any::type_name::<T>(),
            std::any::type_name::<C>()
        );
        let consumed = Arc::try_unwrap(self).unwrap();
        world.insert(consumed.tags, consumed.components);
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { self.write_components.clone() }
    fn write_tags(&self) -> Vec<TagTypeId> { self.write_tags.clone() }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct DeleteEntityCommand(Entity);
impl WorldWritable for DeleteEntityCommand {
    fn write(self: Arc<Self>, world: &mut World) {
        tracing::trace!("delete_entity, type = {}", self.0);
        world.delete(self.0);
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { Vec::with_capacity(0) }
    fn write_tags(&self) -> Vec<TagTypeId> { Vec::with_capacity(0) }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct AddTagCommand<T> {
    entity: Entity,
    #[derivative(Debug = "ignore")]
    tag: T,
}
impl<T> WorldWritable for AddTagCommand<T>
where
    T: Tag,
{
    fn write(self: Arc<Self>, world: &mut World) {
        let consumed = Arc::try_unwrap(self).unwrap();
        tracing::trace!("add_tag, type = {}", std::any::type_name::<T>());
        world.add_tag(consumed.entity, consumed.tag)
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { Vec::with_capacity(0) }
    fn write_tags(&self) -> Vec<TagTypeId> { vec![TagTypeId::of::<T>()] }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
struct RemoveTagCommand<T> {
    entity: Entity,
    _marker: PhantomData<T>,
}
impl<T> WorldWritable for RemoveTagCommand<T>
where
    T: Tag,
{
    fn write(self: Arc<Self>, world: &mut World) {
        tracing::trace!(
            "remove_tag, type = {}, entity = {:?}",
            std::any::type_name::<T>(),
            self.entity
        );
        world.remove_tag::<T>(self.entity)
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { Vec::with_capacity(0) }
    fn write_tags(&self) -> Vec<TagTypeId> { vec![TagTypeId::of::<T>()] }
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
    fn write(self: Arc<Self>, world: &mut World) {
        let consumed = Arc::try_unwrap(self).unwrap();
        tracing::trace!(
            "add_component, type = {}, entity = {:?}",
            std::any::type_name::<C>(),
            consumed.entity
        );
        world
            .add_component::<C>(consumed.entity, consumed.component)
            .unwrap();
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { vec![ComponentTypeId::of::<C>()] }
    fn write_tags(&self) -> Vec<TagTypeId> { Vec::with_capacity(0) }
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
    fn write(self: Arc<Self>, world: &mut World) {
        tracing::trace!("remove_component, type = {}", std::any::type_name::<C>());
        world.remove_component::<C>(self.entity)
    }

    fn write_components(&self) -> Vec<ComponentTypeId> { vec![ComponentTypeId::of::<C>()] }
    fn write_tags(&self) -> Vec<TagTypeId> { Vec::with_capacity(0) }
}

#[allow(clippy::enum_variant_names)]
enum EntityCommand {
    WriteWorld(Arc<dyn WorldWritable>),
    ExecWorld(Arc<dyn Fn(&World)>),
    ExecMutWorld(Arc<dyn Fn(&mut World)>),
}

pub struct EntityBuilder<TS = (), CS = ()> {
    entity: Entity,
    tags: TS,
    components: CS,
}
impl<TS, CS> EntityBuilder<TS, CS>
where
    TS: 'static + Send + ConsFlatten,
    CS: 'static + Send + ConsFlatten,
{
    pub fn with_component<C: Component>(
        self,
        component: C,
    ) -> EntityBuilder<TS, <CS as ConsAppend<C>>::Output>
    where
        CS: ConsAppend<C>,
        <CS as ConsAppend<C>>::Output: ConsFlatten,
    {
        EntityBuilder {
            components: ConsAppend::append(self.components, component),
            entity: self.entity,
            tags: self.tags,
        }
    }

    pub fn with_tag<T: Tag>(self, tag: T) -> EntityBuilder<<TS as ConsAppend<T>>::Output, CS>
    where
        TS: ConsAppend<T>,
        <TS as ConsAppend<T>>::Output: ConsFlatten,
    {
        EntityBuilder {
            tags: ConsAppend::append(self.tags, tag),
            entity: self.entity,
            components: self.components,
        }
    }

    fn build(self, buffer: &mut CommandBuffer)
    where
        <TS as ConsFlatten>::Output: TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        ComponentTupleSet<
            <CS as ConsFlatten>::Output,
            std::iter::Once<<CS as ConsFlatten>::Output>,
        >: ComponentSource,
    {
        buffer
            .commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(InsertBufferedCommand {
                write_components: Vec::default(),
                write_tags: Vec::default(),
                tags: self.tags.flatten(),
                components: IntoComponentSource::into(std::iter::once(self.components.flatten())),
                entities: vec![self.entity],
            })));
    }
}

#[derive(Debug)]
pub enum CommandError {
    EntityBlockFull,
}
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "CommandError") }
}

impl std::error::Error for CommandError {
    fn cause(&self) -> Option<&dyn std::error::Error> { None }
}

#[derive(Default)]
pub struct CommandBuffer {
    commands: AtomicRefCell<VecDeque<EntityCommand>>,
    pub(crate) custom_capacity: Option<usize>,
    pub(crate) free_list: SmallVec<[Entity; 64]>,
    pub(crate) used_list: SmallVec<[Entity; 64]>,
}
// This is safe because only 1 system in 1 execution is only ever accessing a command buffer
// and we garuntee the write operations of a command buffer occur in a safe manner
unsafe impl Send for CommandBuffer {}
unsafe impl Sync for CommandBuffer {}

impl CommandBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        // Pull  free entities from the world.

        Self {
            custom_capacity: Some(capacity),
            free_list: SmallVec::with_capacity(capacity),
            commands: Default::default(),
            used_list: SmallVec::with_capacity(capacity),
        }
    }

    pub fn from_world_with_capacity(world: &mut World, capacity: usize) -> Self {
        // Pull  free entities from the world.

        let free_list =
            SmallVec::from_iter((0..capacity).map(|_| world.entity_allocator.create_entity()));

        Self {
            free_list,
            custom_capacity: Some(capacity),
            commands: Default::default(),
            used_list: SmallVec::with_capacity(capacity),
        }
    }

    pub fn from_world(world: &mut World) -> Self {
        // Pull  free entities from the world.

        let free_list = SmallVec::from_iter(
            (0..world.command_buffer_size()).map(|_| world.entity_allocator.create_entity()),
        );

        Self {
            free_list,
            custom_capacity: None,
            commands: Default::default(),
            used_list: SmallVec::with_capacity(world.command_buffer_size()),
        }
    }

    #[allow(clippy::comparison_chain)]
    pub fn resize(&mut self, world: &mut World, capacity: usize) {
        if self.free_list.len() < capacity {
            (self.free_list.len()..capacity)
                .for_each(|_| self.free_list.push(world.entity_allocator.create_entity()));
        } else if self.free_list.len() > capacity {
            // Free the entities
            (self.free_list.len() - capacity..capacity).for_each(|_| {
                world
                    .entity_allocator
                    .delete_entity(self.free_list.pop().unwrap());
            });
        }
    }

    pub fn write(&mut self, world: &mut World) {
        tracing::trace!("Draining command buffer");

        while let Some(command) = self.commands.get_mut().pop_back() {
            match command {
                EntityCommand::WriteWorld(ptr) => ptr.write(world),
                EntityCommand::ExecMutWorld(closure) => closure(world),
                EntityCommand::ExecWorld(closure) => closure(world),
            }
        }

        // Refill our entity buffer from the world
        if let Some(custom_capacity) = self.custom_capacity {
            self.resize(world, custom_capacity);
        } else {
            self.resize(world, world.command_buffer_size());
        }
    }

    pub fn build_entity(&mut self) -> Result<EntityBuilder<(), ()>, CommandError> {
        let entity = self.create_entity()?;

        Ok(EntityBuilder {
            entity,
            tags: (),
            components: (),
        })
    }

    pub fn create_entity(&mut self) -> Result<Entity, CommandError> {
        let entity = self.free_list.pop().ok_or(CommandError::EntityBlockFull)?;
        self.used_list.push(entity);

        Ok(entity)
    }

    pub fn exec_mut<F>(&self, f: F)
    where
        F: 'static + Fn(&mut World),
    {
        self.commands
            .get_mut()
            .push_front(EntityCommand::ExecMutWorld(Arc::new(f)));
    }

    pub fn insert_writer<W>(&self, writer: W)
    where
        W: 'static + WorldWritable,
    {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(writer)));
    }

    pub fn insert_unbuffered<T, C>(&mut self, tags: T, components: C)
    where
        T: 'static + TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        C: 'static + IntoComponentSource,
    {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(InsertCommand {
                write_components: Vec::default(),
                write_tags: Vec::default(),
                tags,
                components,
            })));
    }

    pub fn insert<T, C>(&mut self, tags: T, components: C) -> Result<Vec<Entity>, CommandError>
    where
        T: 'static + TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        C: 'static + IntoComponentSource,
    {
        let components = components.into();
        if components.len() > self.free_list.len() {
            return Err(CommandError::EntityBlockFull);
        }

        let mut entities = Vec::with_capacity(components.len());
        for _ in 0..components.len() {
            entities.push(self.free_list.pop().ok_or(CommandError::EntityBlockFull)?);
        }

        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(InsertBufferedCommand {
                write_components: Vec::default(),
                write_tags: Vec::default(),
                tags,
                components,
                entities: entities.clone(),
            })));

        Ok(entities)
    }

    pub fn delete(&self, entity: Entity) {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(DeleteEntityCommand(
                entity,
            ))));
    }

    pub fn add_component<C: Component>(&self, entity: Entity, component: C) {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(AddComponentCommand {
                entity,
                component,
            })));
    }

    pub fn remove_component<C: Component>(&self, entity: Entity) {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(
                RemoveComponentCommand {
                    entity,
                    _marker: PhantomData::<C>::default(),
                },
            )));
    }

    pub fn add_tag<T: Tag>(&self, entity: Entity, tag: T) {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(AddTagCommand {
                entity,
                tag,
            })));
    }

    pub fn remove_tag<T: Tag>(&self, entity: Entity) {
        self.commands
            .get_mut()
            .push_front(EntityCommand::WriteWorld(Arc::new(RemoveTagCommand {
                entity,
                _marker: PhantomData::<T>::default(),
            })));
    }

    #[inline]
    pub fn len(&self) -> usize { self.commands.get().len() }

    #[inline]
    pub fn is_empty(&self) -> bool { self.commands.get().len() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);
    #[derive(Default)]
    struct TestResource(pub i32);

    #[test]
    fn create_entity_test() -> Result<(), CommandError> {
        let _ = tracing_subscriber::fmt::try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];
        let components_len = components.len();

        //world.entity_allocator.get_block()
        let mut command = CommandBuffer::from_world(&mut world);
        let entity1 = command.create_entity()?;
        let entity2 = command.create_entity()?;

        command.add_component(entity1, Pos(1., 2., 3.));
        command.add_component(entity2, Pos(4., 5., 6.));

        command.write(&mut world);

        let query = Read::<Pos>::query();

        let mut count = 0;
        for _ in query.iter_entities(&mut world) {
            count += 1;
        }

        assert_eq!(components_len, count);

        Ok(())
    }

    #[test]
    fn simple_write_test() -> Result<(), CommandError> {
        let _ = tracing_subscriber::fmt::try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];
        let components_len = components.len();

        //world.entity_allocator.get_block()
        let mut command = CommandBuffer::from_world(&mut world);
        let _ = command.insert((), components)?;

        // Assert writing checks
        // TODO:
        //assert_eq!(
        //    vec![ComponentTypeId::of::<Pos>(), ComponentTypeId::of::<Vel>()],
        //    command.write_components()
        //);

        command.write(&mut world);

        let query = Read::<Pos>::query();

        let mut count = 0;
        for _ in query.iter_entities(&mut world) {
            count += 1;
        }

        assert_eq!(components_len, count);

        Ok(())
    }
}
