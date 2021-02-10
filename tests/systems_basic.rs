#[cfg(feature = "codegen")]
mod tests {
    use std::fmt::Debug;

    use legion::{
        storage::Component, system, systems::CommandBuffer, world::SubWorld, IntoQuery, Query,
        Read, Resources, Schedule, World, Write,
    };
    use std::{
        fmt::Debug,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

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

    #[test]
    fn with_query() {
        #[system]
        fn basic(_: &mut SubWorld, _: &mut Query<(&u8, &mut i8)>) {}

        Schedule::builder().add_system(basic_system()).build();
    }

    #[test]
    fn with_two_queries() {
        #[system]
        fn basic(
            #[state] a_count: &Arc<AtomicUsize>,
            #[state] b_count: &Arc<AtomicUsize>,
            world: &mut SubWorld,
            a: &mut Query<(&u8, &mut usize)>,
            b: &mut Query<(&usize, &mut bool)>,
        ) {
            a_count.store(a.iter_mut(world).count(), Ordering::SeqCst);
            b_count.store(b.iter_mut(world).count(), Ordering::SeqCst);
        }

        let a_count = Arc::new(AtomicUsize::new(0));
        let b_count = Arc::new(AtomicUsize::new(0));
        let mut schedule = Schedule::builder()
            .add_system(basic_system(a_count.clone(), b_count.clone()))
            .build();

        let mut world = World::default();
        let mut resources = Resources::default();

        world.extend(vec![(1_usize, false), (2_usize, false)]);

        schedule.execute(&mut world, &mut resources);

        assert_eq!(0, a_count.load(Ordering::SeqCst));
        assert_eq!(2, b_count.load(Ordering::SeqCst));
    }
}
