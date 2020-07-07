use legion::*;
use std::collections::HashSet;

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

#[test]
fn insert() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![(4f32, 5u64, 6u16), (4f32, 5u64, 6u16)];
    let entities = world.extend(components);

    assert_eq!(2, entities.len());
}

#[test]
fn get_component() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.extend(components.clone()) {
        entities.push(*e);
    }

    for (i, e) in entities.iter().enumerate() {
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i).map(|(x, _)| x), None),
        }
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn get_component_wrong_type() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let entity = *world.extend(vec![(0f64,)]).get(0).unwrap();

    assert!(world
        .entry_ref(entity)
        .unwrap()
        .get_component::<i32>()
        .is_none());
}

#[test]
fn remove() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.extend(components) {
        entities.push(*e);
    }

    for e in entities.iter() {
        assert_eq!(true, world.contains(*e));
    }

    for e in entities.iter() {
        world.remove(*e);
        assert_eq!(false, world.contains(*e));
    }
}

#[test]
fn delete_all() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.extend(components) {
        entities.push(*e);
    }

    // Check that the entity allocator knows about the entities
    for e in entities.iter() {
        assert_eq!(true, world.contains(*e));
    }

    // Check that the entities are in storage
    let mut query = <(Read<Pos>, Read<Rot>)>::query();
    assert_eq!(2, query.iter(&world).count());

    world.clear();

    // Check that the entity allocator no longer knows about the entities
    for e in entities.iter() {
        assert_eq!(false, world.contains(*e));
    }

    // Check that the entities are removed from storage
    let mut query = <(Read<Pos>, Read<Rot>)>::query();
    assert_eq!(0, query.iter(&world).count());
}

#[test]
fn delete_last() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.extend(components.clone()) {
        entities.push(*e);
    }

    let last = *entities.last().unwrap();
    world.remove(last);
    assert_eq!(false, world.contains(last));

    for (i, e) in entities.iter().take(entities.len() - 1).enumerate() {
        assert_eq!(true, world.contains(*e));
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i).map(|(x, _)| x), None),
        }
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn delete_first() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.extend(components.clone()) {
        entities.push(*e);
    }

    let first = *entities.first().unwrap();

    world.remove(first);
    assert_eq!(false, world.contains(first));

    for (i, e) in entities.iter().skip(1).enumerate() {
        assert_eq!(true, world.contains(*e));
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i + 1).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i + 1).map(|(x, _)| x), None),
        }
        match world.entry_ref(*e).unwrap().get_component() {
            Some(x) => assert_eq!(components.get(i + 1).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i + 1).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn merge() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world_1 = universe.create_world();
    let mut world_2 = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut world_1_entities: Vec<Entity> = Vec::new();
    for e in world_1.extend(components.clone()) {
        world_1_entities.push(*e);
    }

    let mut world_2_entities: Vec<Entity> = Vec::new();
    for e in world_2.extend(components.clone()) {
        world_2_entities.push(*e);
    }

    world_1.move_from(&mut world_2).unwrap();

    for (i, e) in world_2_entities.iter().enumerate() {
        assert!(world_1.contains(*e));

        let (pos, rot) = components.get(i).unwrap();
        assert_eq!(
            pos,
            &world_1.entry(*e).unwrap().get_component().unwrap() as &Pos
        );
        assert_eq!(
            rot,
            &world_1.entry(*e).unwrap().get_component().unwrap() as &Rot
        );
    }
}

#[test]
fn mutate_add_component() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.extend(components).to_vec();

    let mut query_without_scale = <(Read<Pos>, Read<Rot>)>::query();
    let mut query_with_scale = <(Read<Pos>, Read<Rot>, Read<Scale>)>::query();

    assert_eq!(3, query_without_scale.iter(&world).count());
    assert_eq!(0, query_with_scale.iter(&world).count());

    world
        .entry(entities[1])
        .unwrap()
        .add_component(Scale(0.5, 0.5, 0.5));

    assert_eq!(3, query_without_scale.iter(&world).count());
    assert_eq!(1, query_with_scale.iter(&world).count());
}

#[test]
fn mutate_remove_component() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.extend(components).to_vec();

    let mut query_without_rot = Read::<Pos>::query().filter(!component::<Rot>());
    let mut query_with_rot = <(Read<Pos>, Read<Rot>)>::query();

    assert_eq!(0, query_without_rot.iter(&world).count());
    assert_eq!(3, query_with_rot.iter(&world).count());

    world.entry(entities[1]).unwrap().remove_component::<Rot>();

    assert_eq!(1, query_without_rot.iter(&world).count());
    assert_eq!(2, query_with_rot.iter(&world).count());
}

#[test]
#[cfg(feature = "crossbeam-events")]
fn delete_entities_on_drop() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let (tx, rx) = crossbeam_channel::unbounded::<legion::world::Event>();

    let components = vec![(Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3))];

    // Insert the data and store resulting entities in a HashSet
    let mut entities = HashSet::new();
    for entity in world.extend(components) {
        entities.insert(*entity);
    }

    world.subscribe(tx, any());

    //ManuallyDrop::drop(&mut world);
    std::mem::drop(world);

    for e in rx {
        println!("{:?}", e);
        match e {
            legion::world::Event::EntityRemoved(entity, _arch_id) => {
                assert!(entities.remove(&entity));
            }
            _ => {}
        }
    }

    // Verify that no extra entities are included
    assert!(entities.is_empty());
}

// This test repeatedly creates a world with new entities and drops it, reproducing
// https://github.com/TomGillen/legion/issues/92
#[test]
fn lots_of_deletes() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();

    for _ in 0..10000 {
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        let mut world = universe.create_world();
        world.extend(components).to_vec();
    }
}

#[test]
fn iter_entities() {
    let _ = tracing_subscriber::fmt::try_init();

    let universe = Universe::new();
    let mut world = universe.create_world();

    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    // Insert the data and store resulting entities in a HashSet
    let mut entities = HashSet::new();
    for entity in world.extend(components) {
        entities.insert(*entity);
    }

    // Verify that all entities in iter_entities() are included
    let mut all = Entity::query();
    for entity in all.iter(&world) {
        assert!(entities.remove(entity));
    }

    // Verify that no extra entities are included
    assert!(entities.is_empty());
}
