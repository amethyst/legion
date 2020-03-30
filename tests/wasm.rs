#![cfg(all(test, target_arch = "wasm32"))]

use wasm_bindgen_test::*;

mod systems {
    use super::*;
    #[cfg(all(target_arch = "wasm32", not(features = "par-schedule")))]
    mod system {
        use super::*;
        use legion::prelude::*;

        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Pos(f32, f32, f32);
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Vel(f32, f32, f32);

        #[derive(Default)]
        struct TestResource(pub i32);
        #[derive(Default)]
        struct TestResourceTwo(pub i32);
        #[derive(Default)]
        struct TestResourceThree(pub i32);
        #[derive(Default)]
        struct TestResourceFour(pub i32);

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct TestComp(f32, f32, f32);
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct TestCompTwo(f32, f32, f32);
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct TestCompThree(f32, f32, f32);

        #[wasm_bindgen_test]
        fn builder_schedule_execute() {
            let _ = tracing_subscriber::fmt::try_init();

            let universe = Universe::new();
            let mut world = universe.create_world();

            let mut resources = Resources::default();
            resources.insert(TestResource(123));
            resources.insert(TestResourceTwo(123));

            let components = vec![
                (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
            ];

            let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

            for (i, e) in world.insert((), components.clone()).iter().enumerate() {
                if let Some((pos, rot)) = components.get(i) {
                    expected.insert(*e, (*pos, *rot));
                }
            }

            #[derive(Debug, Eq, PartialEq)]
            pub enum TestSystems {
                TestSystemOne,
                TestSystemTwo,
                TestSystemThree,
                TestSystemFour,
            }

            let runs = Arc::new(Mutex::new(Vec::new()));

            let system_one_runs = runs.clone();
            let system_one = SystemBuilder::<()>::new("TestSystem1")
                .read_resource::<TestResource>()
                .with_query(Read::<Pos>::query())
                .with_query(Write::<Vel>::query())
                .build(move |_commands, _world, _resource, _queries| {
                    tracing::trace!("system_one");
                    system_one_runs
                        .lock()
                        .unwrap()
                        .push(TestSystems::TestSystemOne);
                });

            let system_two_runs = runs.clone();
            let system_two = SystemBuilder::<()>::new("TestSystem2")
                .write_resource::<TestResourceTwo>()
                .with_query(Read::<Vel>::query())
                .build(move |_commands, _world, _resource, _queries| {
                    tracing::trace!("system_two");
                    system_two_runs
                        .lock()
                        .unwrap()
                        .push(TestSystems::TestSystemTwo);
                });

            let system_three_runs = runs.clone();
            let system_three = SystemBuilder::<()>::new("TestSystem3")
                .read_resource::<TestResourceTwo>()
                .with_query(Read::<Vel>::query())
                .build(move |_commands, _world, _resource, _queries| {
                    tracing::trace!("system_three");
                    system_three_runs
                        .lock()
                        .unwrap()
                        .push(TestSystems::TestSystemThree);
                });
            let system_four_runs = runs.clone();
            let system_four = SystemBuilder::<()>::new("TestSystem4")
                .write_resource::<TestResourceTwo>()
                .with_query(Read::<Vel>::query())
                .build(move |_commands, _world, _resource, _queries| {
                    tracing::trace!("system_four");
                    system_four_runs
                        .lock()
                        .unwrap()
                        .push(TestSystems::TestSystemFour);
                });

            let order = vec![
                TestSystems::TestSystemOne,
                TestSystems::TestSystemTwo,
                TestSystems::TestSystemThree,
                TestSystems::TestSystemFour,
            ];

            let systems = vec![system_one, system_two, system_three, system_four];

            let mut executor = Executor::new(systems);
            executor.execute(&mut world, &mut resources);

            assert_eq!(*(runs.lock().unwrap()), order);
        }

        #[wasm_bindgen_test]
        fn builder_create_and_execute() {
            let _ = tracing_subscriber::fmt::try_init();

            let universe = Universe::new();
            let mut world = universe.create_world();

            let mut resources = Resources::default();
            resources.insert(TestResource(123));

            let components = vec![
                (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
            ];

            let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

            for (i, e) in world.insert((), components.clone()).iter().enumerate() {
                if let Some((pos, rot)) = components.get(i) {
                    expected.insert(*e, (*pos, *rot));
                }
            }

            let mut system = SystemBuilder::<()>::new("TestSystem")
                .read_resource::<TestResource>()
                .with_query(Read::<Pos>::query())
                .with_query(Read::<Vel>::query())
                .build(move |_commands, world, resource, queries| {
                    assert_eq!(resource.0, 123);
                    let mut count = 0;
                    {
                        for (entity, pos) in queries.0.iter_entities(world) {
                            assert_eq!(expected.get(&entity).unwrap().0, *pos);
                            count += 1;
                        }
                    }

                    assert_eq!(components.len(), count);
                });
            system.prepare(&world);
            system.run(&mut world, &mut resources);
        }

        #[wasm_bindgen_test]
        fn fnmut_stateful_system_test() {
            let _ = tracing_subscriber::fmt::try_init();

            let universe = Universe::new();
            let mut world = universe.create_world();

            let mut resources = Resources::default();
            resources.insert(TestResource(123));

            let components = vec![
                (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
            ];

            let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

            for (i, e) in world.insert((), components.clone()).iter().enumerate() {
                if let Some((pos, rot)) = components.get(i) {
                    expected.insert(*e, (*pos, *rot));
                }
            }

            let mut state = 0;
            let mut system = SystemBuilder::<()>::new("TestSystem")
                .read_resource::<TestResource>()
                .with_query(Read::<Pos>::query())
                .with_query(Read::<Vel>::query())
                .build(move |_, _, _, _| {
                    state += 1;
                });

            system.prepare(&world);
            system.run(&mut world, &mut resources);
        }

        #[wasm_bindgen_test]
        fn system_mutate_archetype() {
            let _ = tracing_subscriber::fmt::try_init();

            let universe = Universe::new();
            let mut world = universe.create_world();
            let mut resources = Resources::default();

            #[derive(Default, Clone, Copy)]
            pub struct Balls(u32);

            let components = vec![
                (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
                (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
            ];

            let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

            for (i, e) in world.insert((), components.clone()).iter().enumerate() {
                if let Some((pos, rot)) = components.get(i) {
                    expected.insert(*e, (*pos, *rot));
                }
            }

            let expected_copy = expected.clone();
            let mut system = SystemBuilder::<()>::new("TestSystem")
                .with_query(<(Read<Pos>, Read<Vel>)>::query())
                .build(move |_, world, _, query| {
                    let mut count = 0;
                    {
                        for (entity, (pos, vel)) in query.iter_entities(world) {
                            assert_eq!(expected_copy.get(&entity).unwrap().0, *pos);
                            assert_eq!(expected_copy.get(&entity).unwrap().1, *vel);
                            count += 1;
                        }
                    }

                    assert_eq!(components.len(), count);
                });

            system.prepare(&world);
            system.run(&mut world, &mut resources);

            world
                .add_component(*(expected.keys().nth(0).unwrap()), Balls::default())
                .unwrap();

            system.prepare(&world);
            system.run(&mut world, &mut resources);
        }

        #[wasm_bindgen_test]
        fn system_mutate_archetype_buffer() {
            let _ = tracing_subscriber::fmt::try_init();

            let universe = Universe::new();
            let mut world = universe.create_world();
            let mut resources = Resources::default();

            #[derive(Default, Clone, Copy)]
            pub struct Balls(u32);

            let components = (0..30000)
                .map(|_| (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)))
                .collect::<Vec<_>>();

            let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

            for (i, e) in world.insert((), components.clone()).iter().enumerate() {
                if let Some((pos, rot)) = components.get(i) {
                    expected.insert(*e, (*pos, *rot));
                }
            }

            let expected_copy = expected.clone();
            let mut system = SystemBuilder::<()>::new("TestSystem")
                .with_query(<(Read<Pos>, Read<Vel>)>::query())
                .build(move |command_buffer, world, _, query| {
                    let mut count = 0;
                    {
                        for (entity, (pos, vel)) in query.iter_entities(world) {
                            assert_eq!(expected_copy.get(&entity).unwrap().0, *pos);
                            assert_eq!(expected_copy.get(&entity).unwrap().1, *vel);
                            count += 1;

                            command_buffer.add_component(entity, Balls::default());
                        }
                    }

                    assert_eq!(components.len(), count);
                });

            system.prepare(&world);
            system.run(&mut world, &mut resources);

            system
                .command_buffer_mut(world.id())
                .unwrap()
                .write(&mut world);

            system.prepare(&world);
            system.run(&mut world, &mut resources);
        }
    }

    #[cfg(all(target_arch = "wasm32", not(features = "par-schedule")))]
    mod schedule {
        use super::*;
        use itertools::sorted;
        use legion::prelude::*;
        use std::sync::{Arc, Mutex};

        #[wasm_bindgen_test]
        fn execute_in_order() {
            let universe = Universe::new();
            let mut world = universe.create_world();

            #[derive(Default)]
            struct Resource;

            let mut resources = Resources::default();
            resources.insert(Resource);

            let order = Arc::new(Mutex::new(Vec::new()));

            let order_clone = order.clone();
            let system_one = SystemBuilder::new("one")
                .write_resource::<Resource>()
                .build(move |_, _, _, _| order_clone.lock().unwrap().push(1usize));
            let order_clone = order.clone();
            let system_two = SystemBuilder::new("two")
                .write_resource::<Resource>()
                .build(move |_, _, _, _| order_clone.lock().unwrap().push(2usize));
            let order_clone = order.clone();
            let system_three = SystemBuilder::new("three")
                .write_resource::<Resource>()
                .build(move |_, _, _, _| order_clone.lock().unwrap().push(3usize));

            let mut schedule = Schedule::builder()
                .add_system(system_one)
                .add_system(system_two)
                .add_system(system_three)
                .build();

            schedule.execute(&mut world, &mut resources);

            let order = order.lock().unwrap();
            let sorted: Vec<usize> = sorted(order.clone()).collect();
            assert_eq!(*order, sorted);
        }

        #[wasm_bindgen_test]
        fn flush() {
            let universe = Universe::new();
            let mut world = universe.create_world();
            let mut resources = Resources::default();

            #[derive(Clone, Copy, Debug, PartialEq)]
            struct TestComp(f32, f32, f32);

            let system_one = SystemBuilder::new("one").build(move |cmd, _, _, _| {
                cmd.insert((), vec![(TestComp(0., 0., 0.),)]);
            });
            let system_two = SystemBuilder::new("two")
                .with_query(Write::<TestComp>::query())
                .build(move |_, world, _, query| assert_eq!(0, query.iter_mut(world).count()));
            let system_three = SystemBuilder::new("three")
                .with_query(Write::<TestComp>::query())
                .build(move |_, world, _, query| assert_eq!(1, query.iter_mut(world).count()));

            let mut schedule = Schedule::builder()
                .add_system(system_one)
                .add_system(system_two)
                .flush()
                .add_system(system_three)
                .build();

            schedule.execute(&mut world, &mut resources);
        }
    }
}
