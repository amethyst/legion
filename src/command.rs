use crate::entity::Entity;
use crate::world::World;

pub enum EntityCommand {
    Delete(Entity),
    Create(Entity),
}

pub struct EntityCommandBuffer {
    commands: Vec<EntityCommand>,
}
