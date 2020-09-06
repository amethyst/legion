#[cfg(feature = "codegen")]
mod tests {
    use legion::{
        storage::Component, system, systems::CommandBuffer, world::SubWorld, Entity, Schedule,
    };
    use std::fmt::Debug;

    #[test]
    fn empty() {
        #[system(for_each)]
        fn for_each(_: &Entity) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_resource() {
        #[system(for_each)]
        fn for_each(_: &Entity, #[resource] _: &usize) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_mut_resource() {
        #[system(for_each)]
        fn for_each(_: &Entity, #[resource] _: &mut usize) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_world() {
        #[system(for_each)]
        fn for_each(_: &Entity, _: &SubWorld) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_mut_world() {
        #[system(for_each)]
        fn for_each(_: &Entity, _: &mut SubWorld) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_cmd() {
        #[system(for_each)]
        fn for_each(_: &Entity, _: &CommandBuffer) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_mut_cmd() {
        #[system(for_each)]
        fn for_each(_: &Entity, _: &mut CommandBuffer) {}

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_components() {
        #[system(for_each)]
        fn for_each(a: &f32, b: &mut usize) {
            println!("{:?} {:?}", a, b);
        }

        Schedule::builder().add_system(for_each_system()).build();
    }

    #[test]
    fn with_generics() {
        #[system(for_each)]
        fn for_each<T: Component + Debug>(t: &T) {
            println!("{:?}", t);
        }

        Schedule::builder()
            .add_system(for_each_system::<usize>())
            .build();
    }
}
