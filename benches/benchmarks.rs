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

fn setup(n: usize) -> World {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    world.insert_from((), (0..n).map(|_| (Position(0.), Rotation(0.))));

    world
}

fn bench_create_delete(c: &mut Criterion) {
    c.bench_function_over_inputs(
        "create-delete",
        |b, count| {
            let mut world = setup(0);
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

fn bench_iter_simple(c: &mut Criterion) {
    c.bench(
        "iter-simple",
        ParameterizedBenchmark::new(
            "query",
            |b, count| {
                let mut world = setup(*count);
                add_background_entities(&mut world, 10000);

                let mut query = <(Read<Position>, Write<Rotation>)>::query();

                b.iter(|| {
                    for (pos, rot) in query.iter(&world) {
                        rot.0 = pos.0;
                    }
                });
            },
            (0..11).map(|i| i * 1000),
        )
        .with_function("cachedquery-on", |b, count| {
            let mut world = setup(*count);
            add_background_entities(&mut world, 10000);

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
            query.set_caching_enabled(false);

            b.iter(|| {
                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-off", |b, count| {
            let mut world = setup(*count);
            add_background_entities(&mut world, 10000);

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
            query.set_caching_enabled(false);

            b.iter(|| {
                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-on-singleuse", |b, count| {
            let mut world = setup(*count);
            add_background_entities(&mut world, 10000);

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
                query.set_caching_enabled(false);

                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-off-singleuse", |b, count| {
            let mut world = setup(*count);
            add_background_entities(&mut world, 10000);

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
                query.set_caching_enabled(false);

                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        }),
    );
}

fn bench_iter_complex(c: &mut Criterion) {
    c.bench(
        "iter-complex",
        ParameterizedBenchmark::new(
            "query",
            |b, count| {
                let mut world = setup(0);
                add_background_entities(&mut world, 10000);

                for i in 0..200 {
                    world.insert_from(
                        (Tag(i as f32),).as_tags(),
                        (0..*count).map(|_| (Position(0.), Rotation(0.))),
                    );
                }

                let mut query = <(Read<Position>, Write<Rotation>)>::query()
                    .filter(!component::<A>() & tag_value(&Tag(2.0)));

                b.iter(|| {
                    for (pos, rot) in query.iter(&world) {
                        rot.0 = pos.0;
                    }
                });
            },
            (0..11).map(|i| i * 1000),
        )
        .with_function("cachedquery-on", |b, count| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..*count).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                .filter(!component::<A>() & tag_value(&Tag(2.0)));
            query.set_caching_enabled(false);

            b.iter(|| {
                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-off", |b, count| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..*count).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                .filter(!component::<A>() & tag_value(&Tag(2.0)));
            query.set_caching_enabled(false);

            b.iter(|| {
                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-on-singleuse", |b, count| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..*count).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                    .filter(!component::<A>() & tag_value(&Tag(2.0)));
                query.set_caching_enabled(false);

                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        })
        .with_function("cachedquery-off-singleuse", |b, count| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..*count).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                    .filter(!component::<A>() & tag_value(&Tag(2.0)));
                query.set_caching_enabled(false);

                for (pos, rot) in query.iter(&world) {
                    rot.0 = pos.0;
                }
            });
        }),
    );
}

fn bench_iter_chunks_simple(c: &mut Criterion) {
    c.bench(
        "iter-chunks-simple",
        Benchmark::new("query", |b| {
            let mut world = setup(10000);
            add_background_entities(&mut world, 10000);

            let mut query = <(Read<Position>, Write<Rotation>)>::query();

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-on", |b| {
            let mut world = setup(10000);
            add_background_entities(&mut world, 10000);

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
            query.set_caching_enabled(false);

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-off", |b| {
            let mut world = setup(10000);
            add_background_entities(&mut world, 10000);

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
            query.set_caching_enabled(false);

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-on-singleuse", |b| {
            let mut world = setup(10000);
            add_background_entities(&mut world, 10000);

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
                query.set_caching_enabled(false);

                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-off-singleuse", |b| {
            let mut world = setup(10000);
            add_background_entities(&mut world, 10000);

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching();
                query.set_caching_enabled(false);

                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        }),
    );
}

fn bench_iter_chunks_complex(c: &mut Criterion) {
    c.bench(
        "iter-chunks-complex",
        Benchmark::new("query", |b| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..10000).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            let mut query = <(Read<Position>, Write<Rotation>)>::query()
                .filter(!component::<A>() & tag_value(&Tag(2.0)));

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-on", |b| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..10000).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                .filter(!component::<A>() & tag_value(&Tag(2.0)));
            query.set_caching_enabled(false);

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-off", |b| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..10000).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                .filter(!component::<A>() & tag_value(&Tag(2.0)));
            query.set_caching_enabled(false);

            b.iter(|| {
                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-on-singleuse", |b| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..10000).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                    .filter(!component::<A>() & tag_value(&Tag(2.0)));
                query.set_caching_enabled(false);

                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        })
        .with_function("cachedquery-off-singleuse", |b| {
            let mut world = setup(0);
            add_background_entities(&mut world, 10000);

            for i in 0..200 {
                world.insert_from(
                    (Tag(i as f32),).as_tags(),
                    (0..10000).map(|_| (Position(0.), Rotation(0.))),
                );
            }

            b.iter(|| {
                let mut query = <(Read<Position>, Write<Rotation>)>::query_with_caching()
                    .filter(!component::<A>() & tag_value(&Tag(2.0)));
                query.set_caching_enabled(false);

                for c in query.iter_chunks(&world) {
                    unsafe {
                        c.components_mut::<Position>()
                            .unwrap()
                            .get_unchecked_mut(0)
                            .0 = 0.0
                    };
                }
            });
        }),
    );
}

criterion_group!(
    basic,
    bench_create_delete,
    bench_iter_simple,
    bench_iter_complex,
    bench_iter_chunks_simple,
    bench_iter_chunks_complex
);
criterion_main!(basic);
