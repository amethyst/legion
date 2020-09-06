#![allow(unused_imports)]

use legion::{system, systems::CommandBuffer, Entity};

#[system(par_for_each)]
fn for_each(_: &Entity, _: &mut CommandBuffer) {}

fn main() {}
