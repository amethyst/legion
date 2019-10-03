use crate::{
    entity::Entity,
    filter::{ChunksetFilterData, Filter},
    storage::{Component, Tag},
    world::{IntoComponentSource, TagLayout, TagSet, World},
};
use std::{marker::PhantomData, sync::Arc};

trait WorldWritable {
    fn write(self, world: &mut World);
}

struct InsertCommand<T, C> {
    tags: T,
    components: C,
}
impl<T, C> WorldWritable for InsertCommand<T, C>
where
    T: TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
    C: IntoComponentSource,
{
    fn write(self, world: &mut World) { world.insert(self.tags, self.components); }
}

struct DeleteEntityCommand(Entity);
impl WorldWritable for DeleteEntityCommand {
    fn write(self, world: &mut World) { world.delete(self.0); }
}

struct AddTagCommand<T>(Entity, T);
impl<T> WorldWritable for AddTagCommand<T>
where
    T: Tag,
{
    fn write(self, world: &mut World) { world.add_tag(self.0, self.1) }
}

struct RemoveTagCommand<T>(Entity, PhantomData<T>);
impl<T> WorldWritable for RemoveTagCommand<T>
where
    T: Tag,
{
    fn write(self, world: &mut World) { world.remove_tag::<T>(self.0) }
}

struct AddComponentCommand<C>(Entity, C);
impl<C> WorldWritable for AddComponentCommand<C>
where
    C: Component,
{
    fn write(self, world: &mut World) { world.add_component::<C>(self.0, self.1) }
}

struct RemoveComponentCommand<C>(Entity, PhantomData<C>);
impl<C> WorldWritable for RemoveComponentCommand<C>
where
    C: Component,
{
    fn write(self, world: &mut World) { world.remove_component::<C>(self.0) }
}

enum EntityCommand {
    WriteWorld(Arc<dyn WorldWritable>),
}

#[derive(Default)]
pub struct CommandBuffer {
    commands: Vec<EntityCommand>,
}
impl CommandBuffer {
    pub fn insert<T, C>(mut self, tags: T, components: C) -> Self
    where
        T: 'static + TagSet + TagLayout + for<'a> Filter<ChunksetFilterData<'a>>,
        C: 'static + IntoComponentSource,
    {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(InsertCommand {
                tags,
                components,
            })));

        self
    }

    pub fn delete(mut self, entity: Entity) -> Self {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(DeleteEntityCommand(
                entity,
            ))));
        self
    }

    pub fn add_component<C: Component>(mut self, entity: Entity, component: C) -> Self {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(AddComponentCommand(
                entity, component,
            ))));

        self
    }

    pub fn remove_component<C: Component>(mut self, entity: Entity) -> Self {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(RemoveComponentCommand(
                entity,
                PhantomData::<C>::default(),
            ))));

        self
    }

    pub fn add_tag<T: Tag>(mut self, entity: Entity, tag: T) -> Self {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(AddTagCommand(
                entity, tag,
            ))));

        self
    }

    pub fn remove_tag<T: Tag>(mut self, entity: Entity) -> Self {
        self.commands
            .push(EntityCommand::WriteWorld(Arc::new(RemoveTagCommand(
                entity,
                PhantomData::<T>::default(),
            ))));

        self
    }
}
