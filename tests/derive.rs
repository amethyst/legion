#![cfg(feature = "derive")]

use legion::prelude::*;
use legion_derive::Uuid;

#[test]
fn derive_basic() {
    #[derive(Uuid)]
    struct Pos(f32, f32, f32);
}
