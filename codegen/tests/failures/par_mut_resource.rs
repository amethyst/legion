#![allow(unused_imports)]

use legion::Entity;
use legion_codegen::system;

#[system(par_for_each)]
fn for_each(_: &Entity, #[resource] _: &mut usize) {}

fn main() {}
