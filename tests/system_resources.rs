#[cfg(feature = "codegen")]
mod tests {
    use legion::system_data::SystemResources;
    use legion::systems::{Fetch, FetchMut};
    use legion::*;

    pub struct TestA(usize);
    pub struct TestB(usize);

    #[test]
    fn basic() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a> {
            test_a: Fetch<'a, TestA>,
            test_b: Fetch<'a, TestB>,
        }

        let test = SystemBuilder::new("test")
            .register_system_resources::<TestSystemResources<'static>>()
            .build(|_, _, test_resources, _| {
                let test_resources: &TestSystemResources = test_resources;
                assert_eq!(test_resources.test_a.0, 1);
                assert_eq!(test_resources.test_b.0, 2);
            });

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(test).build();

        schedule.execute(&mut world, &mut resources);
    }

    #[test]
    fn with_immutable() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a> {
            test_a: Fetch<'a, TestA>,
            test_b: Fetch<'a, TestB>,
        }

        #[system]
        fn basic(#[system_resources] test_resources: &TestSystemResources<'static>) {
            assert_eq!(test_resources.test_a.0, 1);
            assert_eq!(test_resources.test_b.0, 2);
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);
    }

    #[test]
    fn with_mutable() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a> {
            test_a: FetchMut<'a, TestA>,
            test_b: Fetch<'a, TestB>,
        }

        #[system]
        fn basic(#[system_resources] test_resources: &mut TestSystemResources<'static>) {
            test_resources.test_a.0 += test_resources.test_b.0 * 2;
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);

        assert_eq!(resources.get::<TestA>().unwrap().0, 5);
    }

    #[test]
    fn with_tuple_struct() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a>(FetchMut<'a, TestA>, Fetch<'a, TestB>);

        #[system]
        fn basic(#[system_resources] test_resources: &mut TestSystemResources<'static>) {
            (test_resources.0).0 += (test_resources.1).0 * 2;
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);

        assert_eq!(resources.get::<TestA>().unwrap().0, 5);
    }

    #[test]
    fn with_generics() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a, R1, R2: 'static>
        where
            R1: 'static,
        {
            test_a: FetchMut<'a, R1>,
            test_b: Fetch<'a, R2>,
        }

        #[system]
        fn basic(
            #[system_resources] test_resources: &mut TestSystemResources<'static, TestA, TestB>,
        ) {
            test_resources.test_a.0 += test_resources.test_b.0 * 2;
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);

        assert_eq!(resources.get::<TestA>().unwrap().0, 5);
    }

    #[test]
    fn with_several_system_resources() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a, R: 'static>(FetchMut<'a, R>);

        #[system]
        fn basic(
            #[system_resources] test_resources_a: &mut TestSystemResources<'static, TestA>,
            #[system_resources] test_resources_b: &mut TestSystemResources<'static, TestB>,
        ) {
            (test_resources_a.0).0 += (test_resources_b.0).0 * 2;
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);

        assert_eq!(resources.get::<TestA>().unwrap().0, 5);
    }

    #[test]
    fn with_other_resources() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a> {
            test_b: Fetch<'a, TestB>,
        }

        #[system]
        fn basic(
            #[system_resources] test_resources: &TestSystemResources<'static>,
            #[resource] test_a: &mut TestA,
        ) {
            test_a.0 += test_resources.test_b.0 * 2;
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);

        assert_eq!(resources.get::<TestA>().unwrap().0, 5);
    }

    #[test]
    fn with_for_each() {
        #[derive(SystemResources)]
        pub struct TestSystemResources<'a> {
            test_a: Fetch<'a, TestA>,
            test_b: Fetch<'a, TestB>,
        }

        #[system(for_each)]
        fn basic(_: &Entity, #[system_resources] test_resources: &TestSystemResources<'static>) {
            assert_eq!(test_resources.test_a.0, 1);
            assert_eq!(test_resources.test_b.0, 2);
        }

        let mut resources = Resources::default();
        resources.insert(TestA(1));
        resources.insert(TestB(2));

        let mut world = World::default();
        let mut schedule = Schedule::builder().add_system(basic_system()).build();

        schedule.execute(&mut world, &mut resources);
    }
}
