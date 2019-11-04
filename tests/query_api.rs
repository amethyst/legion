use legion::prelude::*;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rot(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scale(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vel(f32, f32, f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Accel(f32, f32, f32);
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
struct Model(u32);
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
struct Static;

// fn create_test_world() -> (
//     World,
//     HashMap<
//         Entity,
//         (
//             Option<Pos>,
//             Option<Rot>,
//             Option<Vel>,
//             Option<Model>,
//             Option<Static>,
//         ),
//     >,
// ) {
//     let universe = Universe::new(None, None);
//     let mut world = universe.create_world();
//     let mut expected: HashMap<
//         Entity,
//         (
//             Option<Pos>,
//             Option<Rot>,
//             Option<Vel>,
//             Option<Model>,
//             Option<Static>,
//         ),
//     > = HashMap::new();

// // pos, rot
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
// for (i, e) in world.insert_from((), data.clone()).iter().enumerate() {
//     let (pos, rot) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), None, None, None));
// }

// // model(1) | pos, rot
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
// for (i, e) in world.insert_from((Model(1),), data.clone()).iter().enumerate() {
//     let (pos, rot) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(1)), None));
// }

// // model(2) | pos, rot
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
// for (i, e) in world.insert_from((Model(2),), data.clone()).iter().enumerate() {
//     let (pos, rot) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(2)), None));
// }

// // static | pos, rot
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
// for (i, e) in world.insert_from((Static,), data.clone()).iter().enumerate() {
//     let (pos, rot) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), None, None, Some(Static)));
// }

// // static, model(1) | pos, rot
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
// for (i, e) in world.insert_from((Static, Model(1)), data.clone()).iter().enumerate() {
//     let (pos, rot) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(1)), Some(Static)));
// }

// // pos, rot, vel
// let data = Vec::from_iter(std::iter::unfold(0f32, |x| {
//     *x += 1.;
//     Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.), Vel(*x + 6., *x + 7., *x + 8.)))
// }).take(1000));
// for (i, e) in world.insert_from((), data.clone()).iter().enumerate() {
//     let (pos, rot, vel) = data.get(i).unwrap();
//     expected.insert(*e, (Some(*pos), Some(*rot), Some(*vel), None, None));
// }

//     (world, expected)
// }

#[test]
fn query_read_entity_data() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Read::<Pos>::query();

    let mut count = 0;
    for (entity, pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_cached_read_entity_data() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Read::<Pos>::query(); //.cached();

    let mut count = 0;
    for (entity, pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;
    }

    assert_eq!(components.len(), count);
}

#[test]
#[cfg(feature = "par-iter")]
fn query_read_entity_data_par() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let count = AtomicUsize::new(0);
    let mut query = Read::<Pos>::query();
    query.par_for_each_chunk(&world, |mut chunk| {
        for (entity, pos) in chunk.iter_entities() {
            assert_eq!(expected.get(&entity).unwrap().0, *pos);
            count.fetch_add(1, Ordering::SeqCst);
        }
    });

    assert_eq!(components.len(), count.load(Ordering::SeqCst));
}

#[test]
#[cfg(feature = "par-iter")]
fn query_read_entity_data_par_foreach() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let count = AtomicUsize::new(0);
    let mut query = Read::<Pos>::query();
    query.par_for_each(&world, |_pos| {
        count.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(components.len(), count.load(Ordering::SeqCst));
}

#[test]
fn query_read_entity_data_tuple() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = <(Read<Pos>, Read<Rot>)>::query();

    let mut count = 0;
    for (entity, (pos, rot)) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        assert_eq!(expected.get(&entity).unwrap().1, *rot);
        count += 1;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_write_entity_data() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Write::<Pos>::query();

    let mut count = 0;
    for (entity, mut pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;

        pos.0 = 0.0;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_write_entity_data_tuple() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = <(Write<Pos>, Write<Rot>)>::query();

    let mut count = 0;
    for (entity, (mut pos, mut rot)) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        assert_eq!(expected.get(&entity).unwrap().1, *rot);
        count += 1;

        pos.0 = 0.0;
        rot.0 = 0.0;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_mixed_entity_data_tuple() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = <(Read<Pos>, Write<Rot>)>::query();

    let mut count = 0;
    for (entity, (pos, mut rot)) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        assert_eq!(expected.get(&entity).unwrap().1, *rot);
        count += 1;

        rot.0 = 0.0;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_partial_match() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = <(Read<Pos>, Write<Rot>)>::query();

    let mut count = 0;
    for (entity, (pos, mut rot)) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        assert_eq!(expected.get(&entity).unwrap().1, *rot);
        count += 1;

        rot.0 = 0.0;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_read_shared_data() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    world.insert(shared, components.clone());

    let mut query = Tagged::<Static>::query();

    let mut count = 0;
    for marker in query.iter(&world) {
        assert_eq!(Static, *marker);
        count += 1;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_on_changed_first() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Read::<Pos>::query().filter(changed::<Pos>() | changed::<Rot>());

    let mut count = 0;
    for (entity, pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;
    }

    assert_eq!(components.len(), count);
}

#[test]
fn query_on_changed_no_changes() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Read::<Pos>::query().filter(changed::<Pos>());

    let mut count = 0;
    for (entity, pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;
    }

    assert_eq!(components.len(), count);

    count = 0;
    for (entity, pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        count += 1;
    }

    assert_eq!(0, count);
}

#[test]
fn query_on_changed_self_changes() {
    let _ = env_logger::builder().is_test(true).try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

    for (i, e) in world.insert(shared, components.clone()).iter().enumerate() {
        if let Some((pos, rot)) = components.get(i) {
            expected.insert(*e, (*pos, *rot));
        }
    }

    let mut query = Write::<Pos>::query().filter(changed::<Pos>());

    let mut count = 0;
    for (entity, mut pos) in query.iter_entities(&world) {
        assert_eq!(expected.get(&entity).unwrap().0, *pos);
        *pos = Pos(1., 1., 1.);
        count += 1;
    }

    assert_eq!(components.len(), count);

    count = 0;
    for pos in query.iter(&world) {
        assert_eq!(Pos(1., 1., 1.), *pos);
        count += 1;
    }

    assert_eq!(components.len(), count);
}
