use criterion::*;
use itertools::*;
use legion::storage::DynamicSingleEntitySource;
use legion::storage::DynamicTagSet;
use std::sync::Arc;

use legion::prelude::*;

#[derive(Copy, Clone, Debug, PartialEq)]
struct A(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct B(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct C(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct D(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct E(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct F(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct Tag(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct Position(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
struct Rotation(f32);

fn create_entities(
    world: &mut World,
    variants: &mut [Box<FnMut(&mut DynamicTagSet, &mut DynamicSingleEntitySource)>],
    num_components: usize,
    count: usize,
) {
    let len_variants = variants.len();
    let components = (0..)
        .flat_map(|step| (0..len_variants).map(move |i| (i + i * step) % len_variants))
        .chunks(num_components);

    for initializers in (&components).into_iter().take(count) {
        let entity = world.insert_from((), Some((A(0.0),)))[0];
        world.mutate_entity(entity, |e| {
            for i in initializers {
                let init = variants.get_mut(i).unwrap();
                let (tags, components) = e.deconstruct();
                init(tags, components);
            }
        });
    }
}

fn add_background_entities(world: &mut World, count: usize) {
    create_entities(
        world,
        &mut [
            Box::new(|_, c| c.add_component(A(0.0))),
            Box::new(|_, c| c.add_component(B(0.0))),
            Box::new(|_, c| c.add_component(C(0.0))),
            Box::new(|t, _| t.set_tag(Arc::new(Tag(0.0)))),
            Box::new(|_, c| c.add_component(D(0.0))),
            Box::new(|t, _| t.set_tag(Arc::new(Tag(1.0)))),
            Box::new(|_, c| c.add_component(E(0.0))),
            Box::new(|t, _| t.set_tag(Arc::new(Tag(2.0)))),
            Box::new(|_, c| c.add_component(F(0.0))),
            Box::new(|t, _| t.set_tag(Arc::new(Tag(3.0)))),
        ],
        5,
        count,
    );
}

fn setup() -> World {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    world.insert_from((), (0..1000).map(|_| (Position(0.), Rotation(0.))));

    world
}

fn bench_create_delete(c: &mut Criterion) {
    c.bench_function_over_inputs(
        "create-delete",
        |b, count| {
            let mut world = setup();
            b.iter(|| {
                let entities = world
                    .insert_from((), (0..*count).map(|_| (Position(0.),)))
                    .to_vec();

                for e in entities {
                    world.delete(e);
                }
            })
        },
        (0..10).map(|i| i * 100),
    );
}

fn bench_process(c: &mut Criterion) {
    c.bench(
        "process",
        ParameterizedBenchmark::new(
            "nocache",
            |b, count| {
                let mut world = setup();
                add_background_entities(&mut world, 10000);
                world.insert_from((), (0..*count).map(|_| (Position(0.), Rotation(0.))));

                let mut query = <(Read<Position>, Write<Rotation>)>::query();

                b.iter(|| {
                    for (pos, rot) in query.iter(&world) {
                        rot.0 = pos.0;
                    }
                });
            },
            (0..10).map(|i| i * 100),
        )
        .with_function("cached", |b, count| {
            let mut world = setup();
            add_background_entities(&mut world, 10000);
            world.insert_from((), (0..*count).map(|_| (Position(0.), Rotation(0.))));

            let mut query = <(Read<Position>, Write<Rotation>)>::query().cached();

            b.iter(|| {
                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        }),
    );
}

criterion_group!(basic, bench_create_delete, bench_process);
criterion_main!(basic);
