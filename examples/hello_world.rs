use legion::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vel(f32, f32, f32);

fn main() {
    // create world
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    // create entities
    world.insert_from(
        (),
        vec![
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
            (Pos(1., 2., 3.), Vel(1., 2., 3.)),
        ],
    );

    // update positions
    let query = <(Write<Pos>, Read<Vel>)>::query();
    for (pos, vel) in query.iter(&world) {
        pos.0 += vel.0;
        pos.1 += vel.1;
        pos.2 += vel.2;
    }

    // update positions in parallel
    let query = <(Write<Pos>, Read<Vel>)>::query();
    query.par_for_each(&world, |(pos, vel)| {
        pos.0 += vel.0;
        pos.1 += vel.1;
        pos.2 += vel.2;
    });
}
