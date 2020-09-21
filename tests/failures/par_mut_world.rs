#![allow(unused_imports)]

use legion::{system, world::SubWorld, Entity};

#[system(par_for_each)]
fn for_each(_: &Entity, _: &mut SubWorld) {}

fn main() {}
