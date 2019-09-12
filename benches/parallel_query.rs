use criterion::*;

use itertools::*;
use legion::prelude::*;
use rayon::join;

#[derive(Copy, Clone, Debug, PartialEq)]
struct A(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct B(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct C(f32);

#[derive(Copy, Clone, Debug)]
enum Variants {
    AB(A, B),
    AC(A, C),
}

fn index(v: Variants) -> u8 {
    match v {
        Variants::AB(_, _) => 0,
        Variants::AC(_, _) => 1,
    }
}

fn generate(i: u8) -> Variants {
    match i {
        0 => Variants::AB(A(0.0), B(0.0)),
        _ => Variants::AC(A(0.0), C(0.0)),
    }
}

fn data(n: usize) -> Vec<Variants> {
    let mut v = Vec::<Variants>::new();

    for _ in 0..n {
        v.push(generate(0));
    }

    for _ in 0..n {
        v.push(generate(1));
    }

    v
}

fn setup(data: &Vec<Variants>) -> World {
    let universe = Universe::new(None, None);
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

fn setup_ideal(data: &Vec<Variants>) -> (Vec<(A, B)>, Vec<(A, C)>) {
    let mut ab = Vec::<(A, B)>::new();
    let mut ac = Vec::<(A, C)>::new();

    for v in data {
        match v {
            Variants::AB(a, b) => ab.push((*a, *b)),
            Variants::AC(a, c) => ac.push((*a, *c)),
        };
    }

    (ab, ac)
}

fn ideal(ab: &mut Vec<(A, B)>, ac: &mut Vec<(A, C)>) {
    for (a, b) in ab.iter_mut() {
        b.0 = a.0;
    }

    for (a, c) in ac.iter_mut() {
        c.0 = a.0;
    }
}

fn sequential(world: &World) {
    for (b, a) in <(Write<B>, Read<A>)>::query().iter(&world) {
        b.0 = a.0;
    }

    for (c, a) in <(Write<C>, Read<A>)>::query().iter(&world) {
        c.0 = a.0;
    }
}

fn parallel(world: &World) {
    join(
        || {
            for (b, a) in <(Write<B>, Read<A>)>::query().iter(&world) {
                b.0 = a.0;
            }
        },
        || {
            for (c, a) in <(Write<C>, Read<A>)>::query().iter(&world) {
                c.0 = a.0;
            }
        },
    );
}

fn par_for_each(world: &World) {
    join(
        || {
            <(Write<B>, Read<A>)>::query().par_for_each(&world, |(b, a)| {
                b.0 = a.0;
            });
        },
        || {
            <(Write<C>, Read<A>)>::query().par_for_each(&world, |(c, a)| {
                c.0 = a.0;
            });
        },
    );
}

fn bench_ordered(c: &mut Criterion) {
    c.bench(
        "concurrent queries",
        ParameterizedBenchmark::new(
            "sequential ideal",
            |b, n| {
                let data = data(*n);
                let (mut ab, mut ac) = setup_ideal(&data);
                b.iter(|| ideal(&mut ab, &mut ac));
            },
            (1..11).map(|i| i * 1000),
        )
        .with_function("sequential", |b, n| {
            let data = data(*n);
            let world = setup(&data);
            b.iter(|| sequential(&world));
        })
        .with_function("parallel", |b, n| {
            let data = data(*n);
            let world = setup(&data);
            join(|| {}, || b.iter(|| parallel(&world)));
        })
        .with_function("par_for_each", |b, n| {
            let data = data(*n);
            let world = setup(&data);
            join(|| {}, || b.iter(|| par_for_each(&world)));
        }),
    );
}

criterion_group!(iterate, bench_ordered);
criterion_main!(iterate);
