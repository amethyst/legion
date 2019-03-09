#![feature(test)]

extern crate test;
use test::Bencher;

use legion::*;

pub const N_POS_PER_VEL: usize = 10;
pub const N_POS: usize = 10000;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Velocity {
    pub dx: f32,
    pub dy: f32,
}

fn build() -> (World, impl Query<View = (Write<Position>, Read<Velocity>)>) {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let pos_with_vel = N_POS / N_POS_PER_VEL;
    let pos_without_vel = N_POS - pos_with_vel;

    world.insert_from(
        (),
        std::iter::repeat((Position { x: 0.0, y: 0.0 },)).take(pos_without_vel),
    );

    world.insert_from(
        (),
        std::iter::repeat((Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }))
            .take(pos_with_vel),
    );

    (world, <(Write<Position>, Read<Velocity>)>::query())
}

#[bench]
fn bench_build(b: &mut Bencher) {
    b.iter(|| build());
}

#[bench]
fn bench_update(b: &mut Bencher) {
    let (world, query) = build();

    b.iter(|| {
        for (pos, vel) in query.iter(&world) {
            pos.x += vel.dx;
            pos.y += vel.dy;
        }
    });
}

#[bench]
fn bench_par_update(b: &mut Bencher) {
    let (world, query) = build();

    b.iter(|| {
        query.par_for_each(&world, |(pos, vel)| {
            pos.x += vel.dx;
            pos.y += vel.dy;
        });
    });
}
