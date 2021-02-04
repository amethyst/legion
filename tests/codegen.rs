use legion::*;
use world::SubWorld;

#[test]
#[cfg(feature = "codegen")]
fn basic_system() {
    #[system]
    fn hello_world() {
        println!("hello world");
    }

    let mut world = World::default();
    let mut schedule = Schedule::builder().add_system(hello_world_system()).build();

    schedule.execute(&mut world, &mut Resources::default());
}

#[test]
#[cfg(feature = "codegen")]
fn for_each_system() {
    #[system(for_each)]
    fn sum(component: &usize, #[resource] total: &mut usize) {
        *total += component;
    }

    let mut world = World::default();
    world.extend(vec![(1usize, true), (2usize, false), (3usize, true)]);

    let mut resources = Resources::default();
    resources.insert(0usize);

    let mut schedule = Schedule::builder().add_system(sum_system()).build();

    schedule.execute(&mut world, &mut resources);
    assert_eq!(*resources.get::<usize>().unwrap(), 6usize);
}

#[test]
#[cfg(feature = "codegen")]
fn query_get() {
    type State = Entity;

    #[system]
    #[read_component(f32)]
    #[read_component(f64)]
    fn sys(world: &mut SubWorld, #[state] entity: &State) {
        let mut query = <(&f32, &f64)>::query();
        let _ = query.get_mut(world, *entity);
    }

    let mut world = World::default();
    let entity = world.push(());

    let mut schedule = Schedule::builder().add_system(sys_system(entity)).build();

    let mut resources = Resources::default();

    schedule.execute(&mut world, &mut resources);
}
