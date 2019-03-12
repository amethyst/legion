#[macro_use]
extern crate criterion;

use criterion::Criterion;

use itertools::*;
use legion::prelude::*;

#[derive(Copy, Clone, Debug, PartialEq)]
struct A(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct B(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct C(f32);

#[derive(Copy, Clone, Debug)]
enum Variants {
    AB(A, B),
    A(A),
    AC(A, C),
}

fn index(v: Variants) -> u8 {
    match v {
        Variants::AB(_, _) => 0,
        Variants::A(_) => 1,
        Variants::AC(_, _) => 2,
    }
}

fn generate(i: u8) -> Variants {
    match i {
        0 => Variants::AB(A(0.0), B(0.0)),
        1 => Variants::A(A(0.0)),
        _ => Variants::AC(A(0.0), C(0.0)),
    }
}

const N: usize = 10000;

fn sorted(load: usize) -> Vec<Variants> {
    let mut v = Vec::<Variants>::new();

    for _ in 0..N {
        v.push(generate(0));
    }

    for i in 1..3 {
        for _ in 0..(N * load) {
            v.push(generate(i));
        }
    }

    v
}

fn mixed(load: usize) -> Vec<Variants> {
    let mut v = Vec::<Variants>::new();

    let mut other = 0usize;
    for i in 0..(N + N * load) {
        if i % (load + 1) == 0 {
            v.push(generate(0));
        } else {
            v.push(generate((other % 2) as u8 + 1));
            other += 1;
        }
    }

    v
}

fn setup(data: &Vec<Variants>) -> World {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    for (i, group) in &data.into_iter().group_by(|x| index(**x)) {
        match i {
            0 => world.insert_from(
                (),
                group.map(|x| {
                    if let Variants::AB(a, b) = x {
                        (*a, *b)
                    } else {
                        panic!();
                    }
                }),
            ),
            1 => world.insert_from(
                (),
                group.map(|x| {
                    if let Variants::A(a) = x {
                        (*a,)
                    } else {
                        panic!();
                    }
                }),
            ),
            _ => world.insert_from(
                (),
                group.map(|x| {
                    if let Variants::AC(a, c) = x {
                        (*a, *c)
                    } else {
                        panic!();
                    }
                }),
            ),
        };
    }

    world
}

fn process(world: &World) {
    for (a, b) in <(Write<A>, Read<B>)>::query().iter(&world) {
        a.0 = b.0;
    }
}

fn bench_ordered(c: &mut Criterion) {
    let load_factors = 0..10usize;

    c.bench_function_over_inputs(
        "iterate-ordered load",
        |b, load| {
            let data = sorted(*load);
            let world = setup(&data);
            b.iter(|| process(&world));
        },
        load_factors,
    );
}

fn bench_interleaved(c: &mut Criterion) {
    let load_factors = 0..10usize;

    c.bench_function_over_inputs(
        "iterate-interleaved load",
        |b, load| {
            let data = mixed(*load);
            let world = setup(&data);
            b.iter(|| process(&world));
        },
        load_factors,
    );
}

criterion_group!(iterate, bench_ordered, bench_interleaved);
criterion_main!(iterate);
