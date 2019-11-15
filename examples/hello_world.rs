use legion::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vel(f32, f32, f32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stages {
    Update,
    Draw,
}

impl Stage for Stages {}

impl std::fmt::Display for Stages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stages::Update => write!(f, "update"),
            Stages::Draw => write!(f, "draw"),
        }
    }
}

fn main() {
    let _ = tracing_subscriber::fmt::try_init();

    // create world
    let universe = Universe::new();
    let mut world = universe.create_world();

    // create entities
    world.insert(
        (),
        vec![
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
        ],
    );

    // update positions
    let mut query = <(Write<Pos>, Read<Vel>)>::query();
    for (mut pos, vel) in query.iter(&mut world) {
        pos.0 += vel.0;
        pos.1 += vel.1;
        pos.2 += vel.2;
    }

    // update positions using a system
    let update_positions = SystemBuilder::new("update_positions")
        .with_query(<(Write<Pos>, Read<Vel>)>::query())
        .build(|_, mut world, _, query| {
            for (mut pos, vel) in query.iter(&mut world) {
                pos.0 += vel.0;
                pos.1 += vel.1;
                pos.2 += vel.2;
            }
        });

    let mut scheduler = SystemScheduler::new();
    scheduler.add_system(Stages::Update, update_positions);

    scheduler.execute(&mut world);
}
