#[cfg(feature = "codegen")]
mod tests {
    use legion::{
        storage::Component, system, systems::CommandBuffer, world::SubWorld, IntoQuery, Read,
        Schedule, Write,
    };
    use std::fmt::Debug;

    #[test]
    fn empty() {
        #[system]
        fn basic() {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_resource() {
        #[system]
        fn basic(#[resource] _: &usize) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_mut_resource() {
        #[system]
        fn basic(#[resource] _: &mut usize) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_not_sendsync_resource() {
        struct NotSync(*const usize);

        #[system]
        fn basic(#[resource] _: &NotSync) {}

        Schedule::builder().add_thread_local(basic_system()).build();
    }

    #[test]
    fn with_mut_not_sendsync_resource() {
        struct NotSync(*const usize);

        #[system]
        fn basic(#[resource] _: &mut NotSync) {}

        Schedule::builder().add_thread_local(basic_system()).build();
    }

    #[test]
    fn with_world() {
        #[system]
        #[read_component(usize)]
        fn basic(_: &SubWorld) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_mut_world() {
        #[system]
        #[read_component(usize)]
        fn basic(_: &mut SubWorld) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_cmd() {
        #[system]
        fn basic(_: &CommandBuffer) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_cmd_full_path() {
        #[system]
        fn basic(_: &legion::systems::CommandBuffer) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_mut_cmd() {
        #[system]
        fn basic(_: &mut CommandBuffer) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_components() {
        #[system]
        #[read_component(f32)]
        #[write_component(usize)]
        fn basic(world: &mut SubWorld) {
            let mut query = <(Read<f32>, Write<usize>)>::query();
            for (a, b) in query.iter_mut(world) {
                println!("{:?} {:?}", a, b);
            }
        }

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_generics() {
        #[system]
        #[read_component(T)]
        fn basic<T: Component + Debug>(world: &mut SubWorld) {
            let mut query = Read::<T>::query();
            for t in query.iter_mut(world) {
                println!("{:?}", t);
            }
        }

        Schedule::builder()
            .add_system(basic_system::<usize>())
            .build();
    }

    #[test]
    fn with_generics_with_where() {
        #[system]
        #[read_component(T)]
        fn basic<T>(world: &mut SubWorld)
        where
            T: Component + Debug,
        {
            let mut query = Read::<T>::query();
            for t in query.iter_mut(world) {
                println!("{:?}", t);
            }
        }

        Schedule::builder()
            .add_system(basic_system::<usize>())
            .build();
    }

    #[test]
    fn with_state() {
        #[system]
        fn basic<T: 'static>(#[state] _: &T) {}

        Schedule::builder().add_system(basic_system(false)).build();
    }

    #[test]
    fn with_mut_state() {
        #[system]
        fn basic<T: 'static>(#[state] _: &mut T) {}

        Schedule::builder().add_system(basic_system(false)).build();
    }
}
