#![allow(unused_imports)]

use legion::{system, Entity};

#[system(par_for_each)]
fn for_each(_: &Entity, #[resource] _: &mut usize) {}

fn main() {}
