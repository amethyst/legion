use criterion::*;
use legion::*;

fn bench_insert_zero_baseline(c: &mut Criterion) {
    c.bench_function("insert_zero_baseline", |b| {
        b.iter(|| {
            //let universe = Universe::new();
            //let mut world = universe.create_world();
            let components: Vec<isize> = (0..10000).collect();
            criterion::black_box(components);
        });
    });
}

fn bench_insert_one_baseline(c: &mut Criterion) {
    c.bench_function("insert_one_baseline", |b| {
        b.iter(|| {
            let mut world = World::default();
            let components: Vec<isize> = (0..10000).collect();
            criterion::black_box(components);

            world.extend(vec![(1usize,)]);
        });
    });
}

fn bench_insert_unbatched(c: &mut Criterion) {
    c.bench_function("insert_unbatched", |b| {
        b.iter(|| {
            let mut world = World::default();
            let components: Vec<isize> = (0..10000).collect();

            for component in components {
                world.extend(vec![(component,)]);
            }
        });
    });
}

fn bench_insert_batched(c: &mut Criterion) {
    c.bench(
        "insert_batched",
        ParameterizedBenchmark::new(
            "counts",
            |b, n| {
                b.iter(|| {
                    let mut world = World::default();
                    let components: Vec<(isize,)> = (0..*n).map(|i| (i,)).collect();

                    world.extend(components);
                });
            },
            (1..11).map(|i| i * 1000),
        ),
    );
}

criterion_group!(
    basic,
    bench_insert_zero_baseline,
    bench_insert_one_baseline,
    bench_insert_unbatched,
    bench_insert_batched,
);
criterion_main!(basic);
