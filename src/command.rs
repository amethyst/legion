use crate::{
    entity::{Entity, EntityBlock},
    filter::{ChunksetFilterData, Filter},
    storage::{Component, ComponentTypeId, Tag, TagTypeId},
    world::{IntoComponentSource, TagLayout, TagSet, World},
};
use derivative::Derivative;
use std::{marker::PhantomData, sync::Arc};

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
        log::trace!("Adding component: {}", std::any::type_name::<C>());
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

pub struct EntityBuilder<'a> {
    entity: Entity,
    buffer: &'a mut CommandBuffer,
}
impl<'a> EntityBuilder<'a> {
    fn new(entity: Entity, buffer: &'a mut CommandBuffer) -> Self { Self { entity, buffer } }

    pub fn with_component<C: Component>(self, component: C) -> Self {
        let buffer = self.buffer;
        let entity = self.entity;
        buffer.add_component(self.entity, component);

        Self { entity, buffer }
    }

    pub fn with_tag<T: Tag>(self, tag: T) -> Self {
        let buffer = self.buffer;
        let entity = self.entity;
        buffer.add_tag(self.entity, tag);

        Self { entity, buffer }
    }

    /// This releases the borrow on the CommandBuffer
    pub fn build(self) {}
}

#[allow(clippy::enum_variant_names)]
enum EntityCommand {
    WriteWorld(Arc<dyn WorldWritable>),
    ExecWorld(Arc<dyn Fn(&World)>),
    ExecMutWorld(Arc<dyn Fn(&mut World)>),
}

#[derive(Default)]
pub struct CommandBuffer {
    commands: Vec<EntityCommand>,
    entity_block: Option<EntityBlock>,
}
// This is safe because only 1 system in 1 execution is only ever accessing a command buffer
// and we garuntee the write operations of a command buffer occur in a safe manner
unsafe impl Send for CommandBuffer {}
unsafe impl Sync for CommandBuffer {}

impl CommandBuffer {
    pub fn build_entity(&mut self) -> Option<EntityBuilder<'_>> {
        let entity = self.create_entity()?;
        Some(EntityBuilder::new(entity, self))
    }

    // Prepares this command buffer with a new entity block ???
    fn swap_block(&mut self, world: &mut World) {
        let old_block = self
            .entity_block
            .replace(world.entity_allocator.get_block());
        if let Some(block) = old_block {
            world.entity_allocator.push_block(block);
        }
    }

    pub fn create_entity(&mut self) -> Option<Entity> {
        if let Some(block) = self.entity_block.as_mut() {
            block.allocate()
        } else {
            None
        }
    }
    pub fn create_entities(&mut self, count: usize) -> Vec<Option<Entity>> {
        (0..count).map(|_| self.create_entity()).collect()
    }

    pub fn write(&mut self, world: &mut World) {
        log::trace!("Performing drain");
        self.commands.drain(..).for_each(|command| match command {
            EntityCommand::WriteWorld(ptr) => ptr.write(world),
            EntityCommand::ExecMutWorld(closure) => closure(world),
            EntityCommand::ExecWorld(closure) => closure(world),
        })
    }

    pub fn exec_mut<F>(&mut self, f: F)
    where
        F: 'static + Fn(&mut World),
    {
        self.commands.push(EntityCommand::ExecMutWorld(Arc::new(f)));
    }

    pub fn insert_writer<W>(&mut self, writer: W)
    where
        W: 'static + WorldWritable,
    {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(writer)));
    }

    pub fn insert<T, C>(&mut self, tags: T, components: C)
    where
        T: 'static + TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        C: 'static + IntoComponentSource,
    {
        // TODO: how do we do this?
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(InsertCommand {
                write_components: Vec::default(),
                write_tags: Vec::default(),
                tags,
                components,
            })));
    }

    pub fn delete(&mut self, entity: Entity) {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(DeleteEntityCommand(
                entity,
            ))));
    }

    pub fn add_component<C: Component>(&mut self, entity: Entity, component: C) {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(AddComponentCommand {
                entity,
                component,
            })));
    }

    pub fn remove_component<C: Component>(&mut self, entity: Entity) {
        self.commands.push(EntityCommand::WriteWorld(Arc::new(
            RemoveComponentCommand {
                entity,
                _marker: PhantomData::<C>::default(),
            },
        )));
    }

    pub fn add_tag<T: Tag>(&mut self, entity: Entity, tag: T) {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(AddTagCommand {
                entity,
                tag,
            })));
    }

    pub fn remove_tag<T: Tag>(&mut self, entity: Entity) {
        self.commands
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
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];
        let components_len = components.len();
        let mut command = CommandBuffer::default();
        command.insert((), components);

        // Assert writing checks
        // TODO:
        //assert_eq!(
        //    vec![ComponentTypeId::of::<Pos>(), ComponentTypeId::of::<Vel>()],
        //    command.write_components()
        //);

        command.write(&mut world);

        let mut query = Read::<Pos>::query();

        let mut count = 0;
        for (_, _) in query.iter_entities(&world) {
            //assert_eq!(expected.get(&entity).unwrap().0, *pos);
            count += 1;
        }

        assert_eq!(components_len, count);
    }
}
