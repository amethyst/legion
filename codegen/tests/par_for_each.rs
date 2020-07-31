use legion::{storage::Component, systems::CommandBuffer, world::SubWorld, Entity, Schedule};
use legion_codegen::system;
use std::fmt::Debug;

#[test]
fn empty() {
    #[system(par_for_each)]
    fn for_each(_: &Entity) {}

    Schedule::builder().add_system(for_each_system()).build();
}

#[test]
fn with_resource() {
    #[system(par_for_each)]
    fn for_each(_: &Entity, #[resource] _: &usize) {}

    Schedule::builder().add_system(for_each_system()).build();
}

#[test]
fn with_world() {
    #[system(par_for_each)]
    fn for_each(_: &Entity, _: &SubWorld) {}

    Schedule::builder().add_system(for_each_system()).build();
}

#[test]
fn with_cmd() {
    #[system(par_for_each)]
    fn for_each(_: &Entity, _: &CommandBuffer) {}

    Schedule::builder().add_system(for_each_system()).build();
}

#[test]
fn with_components() {
    #[system(par_for_each)]
    fn for_each(a: &f32, b: &mut usize) {
        println!("{:?} {:?}", a, b);
    }

    Schedule::builder().add_system(for_each_system()).build();
}

#[test]
fn with_generics() {
    #[system(par_for_each)]
    fn for_each<T: Component + Debug>(t: &T) {
        println!("{:?}", t);
    }

    Schedule::builder()
        .add_system(for_each_system::<usize>())
        .build();
}
