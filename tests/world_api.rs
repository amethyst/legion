use hydro::*;
use std::num::Wrapping;

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
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (1usize, 2f32, 3u16);
    let components = vec![(4f32, 5u64, 6u16), (4f32, 5u64, 6u16)];
    let entities = world.insert_from(shared, components);

    assert_eq!(2, entities.len());
}

#[test]
fn get_component() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.insert_from(shared, components.clone()) {
        entities.push(*e);
    }

    for (i, e) in entities.iter().enumerate() {
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i).map(|(x, _)| x), None),
        }
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn get_component_empty_world() {
    let universe = Universe::new(None);
    let world = universe.create_world();

    assert_eq!(None, world.component::<i32>(Entity::new(0, Wrapping(0))));
}

#[test]
fn get_component_wrong_type() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let entity = *world.insert_from((), vec![(0f64,)]).get(0).unwrap();

    assert_eq!(None, world.component::<i32>(entity));
}

#[test]
fn get_shared() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.insert_from(shared, components.clone()) {
        entities.push(*e);
    }

    for e in entities.iter() {
        assert_eq!(Some(&Static), world.shared(*e));
        assert_eq!(Some(&Model(5)), world.shared(*e));
    }
}

#[test]
fn get_shared_empty_world() {
    let universe = Universe::new(None);
    let world = universe.create_world();

    assert_eq!(None, world.shared::<i32>(Entity::new(0, Wrapping(0))));
}

#[test]
fn get_shared_wrong_type() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let entity = *world.insert_from((Static,), vec![(0f64,)]).get(0).unwrap();

    assert_eq!(None, world.shared::<Model>(entity));
}

#[test]
fn delete() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.insert_from(shared, components.clone()) {
        entities.push(*e);
    }

    for e in entities.iter() {
        assert_eq!(true, world.is_alive(e));
    }

    for e in entities.iter() {
        world.delete(*e);
        assert_eq!(false, world.is_alive(e));
    }
}

#[test]
fn delete_last() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.insert_from(shared, components.clone()) {
        entities.push(*e);
    }

    let last = *entities.last().unwrap();
    world.delete(last);
    assert_eq!(false, world.is_alive(&last));

    for (i, e) in entities.iter().take(entities.len() - 1).enumerate() {
        assert_eq!(true, world.is_alive(e));
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i).map(|(x, _)| x), None),
        }
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn delete_first() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut entities: Vec<Entity> = Vec::new();
    for e in world.insert_from(shared, components.clone()) {
        entities.push(*e);
    }

    let first = *entities.first().unwrap();

    world.delete(first);
    assert_eq!(false, world.is_alive(&first));

    for (i, e) in entities.iter().skip(1).enumerate() {
        assert_eq!(true, world.is_alive(e));
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i + 1).map(|(x, _)| x), Some(&x as &Pos)),
            None => assert_eq!(components.get(i + 1).map(|(x, _)| x), None),
        }
        match world.component(*e) {
            Some(x) => assert_eq!(components.get(i + 1).map(|(_, x)| x), Some(&x as &Rot)),
            None => assert_eq!(components.get(i + 1).map(|(_, x)| x), None),
        }
    }
}

#[test]
fn merge() {
    let universe = Universe::new(None);
    let mut world_1 = universe.create_world();
    let mut world_2 = universe.create_world();

    let shared = (Static, Model(5));
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let mut world_1_entities: Vec<Entity> = Vec::new();
    for e in world_1.insert_from(shared, components.clone()) {
        world_1_entities.push(*e);
    }

    let mut world_2_entities: Vec<Entity> = Vec::new();
    for e in world_2.insert_from(shared, components.clone()) {
        world_2_entities.push(*e);
    }

    world_1.merge(world_2);

    for (i, e) in world_2_entities.iter().enumerate() {
        assert!(world_1.is_alive(e));

        let (pos, rot) = components.get(i).unwrap();
        assert_eq!(pos, &world_1.component(*e).unwrap() as &Pos);
        assert_eq!(rot, &world_1.component(*e).unwrap() as &Rot);
    }
}
