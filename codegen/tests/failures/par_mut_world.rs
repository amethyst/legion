#![allow(unused_imports)]

use legion::{world::SubWorld, Entity};
use legion_codegen::system;

#[system(par_for_each)]
fn for_each(_: &Entity, _: &mut SubWorld) {}

fn main() {}
