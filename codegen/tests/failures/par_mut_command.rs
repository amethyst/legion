#![allow(unused_imports)]

use legion::{systems::CommandBuffer, Entity};
use legion_codegen::system;

#[system(par_for_each)]
fn for_each(_: &Entity, _: &mut CommandBuffer) {}

fn main() {}
