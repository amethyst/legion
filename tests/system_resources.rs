use crate::system_data::SystemResources;
use legion::systems::Fetch;
use legion::*;

#[test]
#[cfg(feature = "codegen")]
fn basic_system() {
    pub struct TestA(usize);
    pub struct TestB(usize);

    #[derive(SystemResources)]
    pub struct TestSystemResources<'a> {
        test_a: Fetch<'a, TestA>,
        test_b: Fetch<'a, TestB>,
    }

    let test = SystemBuilder::new("test")
        .register_system_resources::<TestSystemResources<'static>>()
        .build(|_, _, test_resources, _| {
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
