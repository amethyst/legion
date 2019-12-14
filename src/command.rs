use crate::{
    cons::{ConsAppend, ConsFlatten},
    entity::{Entity, EntityBlock},
    filter::{ChunksetFilterData, Filter},
    storage::{Component, ComponentTypeId, Tag, TagTypeId},
    world::{ComponentSource, ComponentTupleSet, IntoComponentSource, TagLayout, TagSet, World},
};
use bit_set::BitSet;

use derivative::Derivative;
use std::{marker::PhantomData, sync::Arc};

#[cfg(feature = "par-schedule")]
use crossbeam_queue::SegQueue;

#[cfg(not(feature = "par-schedule"))]
use crate::borrow::{AtomicRefCell, RefMut};

pub trait WorldWritable {
    fn write(self: Arc<Self>, world: &mut World);

    fn write_components(&self) -> Vec<ComponentTypeId>;
    fn write_tags(&self) -> Vec<TagTypeId>;
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
    fn write(self: Arc<Self>, world: &mut World) { world.delete(self.0); }

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
    fn write(self: Arc<Self>, world: &mut World) { world.remove_tag::<T>(self.entity) }

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
        world.add_component::<C>(consumed.entity, consumed.component)
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
    fn write(self: Arc<Self>, world: &mut World) { world.remove_component::<C>(self.entity) }

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

    fn insert(self, world: &mut World)
    where
        <TS as ConsFlatten>::Output: TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        ComponentTupleSet<
            <CS as ConsFlatten>::Output,
            std::iter::Once<<CS as ConsFlatten>::Output>,
        >: ComponentSource,
    {
        world.insert_buffered(
            self.entity,
            self.tags.flatten(),
            std::iter::once(self.components.flatten()),
        );
    }
}

#[derive(Default)]
pub struct CommandBuffer {
    #[cfg(feature = "par-schedule")]
    commands: SegQueue<EntityCommand>,
    #[cfg(not(feature = "par-schedule"))]
    commands: AtomicRefCell<Vec<EntityCommand>>,
    block: Option<EntityBlock>,
    used_entities: BitSet,
}
// This is safe because only 1 system in 1 execution is only ever accessing a command buffer
// and we garuntee the write operations of a command buffer occur in a safe manner
unsafe impl Send for CommandBuffer {}
unsafe impl Sync for CommandBuffer {}

pub enum CommandError {
    EntityBlockFull,
}

impl CommandBuffer {
    #[cfg(not(feature = "par-schedule"))]
    #[inline]
    fn get_commands(&self) -> RefMut<Vec<EntityCommand>> { self.commands.get_mut() }

    #[cfg(feature = "par-schedule")]
    #[inline]
    fn get_commands(&self) -> &SegQueue<EntityCommand> { &self.commands }

    pub fn write(&self, world: &mut World) {
        tracing::trace!("Draining command buffer");
        #[cfg(feature = "par-schedule")]
        {
            while let Ok(command) = self.get_commands().pop() {
                match command {
                    EntityCommand::WriteWorld(ptr) => ptr.write(world),
                    EntityCommand::ExecMutWorld(closure) => closure(world),
                    EntityCommand::ExecWorld(closure) => closure(world),
                }
            }
        }

        #[cfg(not(feature = "par-schedule"))]
        {
            while let Some(command) = self.get_commands().pop() {
                match command {
                    EntityCommand::WriteWorld(ptr) => ptr.write(world),
                    EntityCommand::ExecMutWorld(closure) => closure(world),
                    EntityCommand::ExecWorld(closure) => closure(world),
                }
            }
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
        let entity = self
            .block
            .as_mut()
            .expect("CommandBuffer::create_entity called when EntityBlock not assigned.")
            .allocate()
            .ok_or(CommandError::EntityBlockFull)?;

        self.used_entities.insert(entity.index() as usize);

        Ok(entity)
    }

    pub fn exec_mut<F>(&self, f: F)
    where
        F: 'static + Fn(&mut World),
    {
        self.get_commands()
            .push(EntityCommand::ExecMutWorld(Arc::new(f)));
    }

    pub fn insert_writer<W>(&self, writer: W)
    where
        W: 'static + WorldWritable,
    {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(writer)));
    }

    pub fn insert<T, C>(&self, tags: T, components: C)
    where
        T: 'static + TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        C: 'static + IntoComponentSource,
    {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(InsertCommand {
                write_components: Vec::default(),
                write_tags: Vec::default(),
                tags,
                components,
            })));
    }

    pub fn delete(&self, entity: Entity) {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(DeleteEntityCommand(
                entity,
            ))));
    }

    pub fn add_component<C: Component>(&self, entity: Entity, component: C) {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(AddComponentCommand {
                entity,
                component,
            })));
    }

    pub fn remove_component<C: Component>(&self, entity: Entity) {
        self.get_commands().push(EntityCommand::WriteWorld(Arc::new(
            RemoveComponentCommand {
                entity,
                _marker: PhantomData::<C>::default(),
            },
        )));
    }

    pub fn add_tag<T: Tag>(&self, entity: Entity, tag: T) {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(AddTagCommand {
                entity,
                tag,
            })));
    }

    pub fn remove_tag<T: Tag>(&self, entity: Entity) {
        self.get_commands()
            .push(EntityCommand::WriteWorld(Arc::new(RemoveTagCommand {
                entity,
                _marker: PhantomData::<T>::default(),
            })));
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);
    #[derive(Default)]
    struct TestResource(pub i32);

    #[test]
    fn simple_write_test() {
        let _ = tracing_subscriber::fmt::try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];
        let components_len = components.len();

        //world.entity_allocator.get_block()
        let command = CommandBuffer::default();
        command.insert((), components);

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
    }
}
