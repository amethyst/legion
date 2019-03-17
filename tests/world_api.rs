use legion::prelude::*;
use std::sync::Arc;

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
    let entities = world.insert_from(shared.as_tags(), components);

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
    for e in world.insert_from(shared.as_tags(), components.clone()) {
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
    for e in world.insert_from(shared.as_tags(), components.clone()) {
        entities.push(*e);
    }

    for e in entities.iter() {
        assert_eq!(Some(&Static), world.tag(*e));
        assert_eq!(Some(&Model(5)), world.tag(*e));
    }
}

#[test]
fn get_shared_wrong_type() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let entity = *world
        .insert_from((Static,).as_tags(), vec![(0f64,)])
        .get(0)
        .unwrap();

    assert_eq!(None, world.tag::<Model>(entity));
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
    for e in world.insert_from(shared.as_tags(), components.clone()) {
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
    for e in world.insert_from(shared.as_tags(), components.clone()) {
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
    for e in world.insert_from(shared.as_tags(), components.clone()) {
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
    for e in world_1.insert_from(shared.as_tags(), components.clone()) {
        world_1_entities.push(*e);
    }

    let mut world_2_entities: Vec<Entity> = Vec::new();
    for e in world_2.insert_from(shared.as_tags(), components.clone()) {
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

#[test]
fn mutate_add_component() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5)).as_tags();
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.insert_from(shared, components).to_vec();

    let query_without_scale = <(Read<Pos>, Read<Rot>)>::query();
    let query_with_scale = <(Read<Pos>, Read<Rot>, Read<Scale>)>::query();

    assert_eq!(3, query_without_scale.iter(&world).count());
    assert_eq!(0, query_with_scale.iter(&world).count());

    world.mutate_entity(*entities.get(1).unwrap(), |_, components| {
        components.add_component(Scale(0.5, 0.5, 0.5))
    });

    assert_eq!(3, query_without_scale.iter(&world).count());
    assert_eq!(1, query_with_scale.iter(&world).count());
}

#[test]
fn mutate_remove_component() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Static, Model(5)).as_tags();
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.insert_from(shared, components).to_vec();

    let query_without_rot = Read::<Pos>::query().filter(!component::<Rot>());
    let query_with_rot = <(Read<Pos>, Read<Rot>)>::query();

    assert_eq!(0, query_without_rot.iter(&world).count());
    assert_eq!(3, query_with_rot.iter(&world).count());

    world.mutate_entity(*entities.get(1).unwrap(), |_, components| {
        components.remove_component::<Rot>();
    });

    assert_eq!(1, query_without_rot.iter(&world).count());
    assert_eq!(2, query_with_rot.iter(&world).count());
}

#[test]
fn mutate_add_tag() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Model(5),).as_tags();
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.insert_from(shared, components).to_vec();

    let query_without_static = <(Read<Pos>, Read<Rot>)>::query();
    let query_with_static = <(Read<Pos>, Read<Rot>, Tagged<Static>)>::query();

    assert_eq!(3, query_without_static.iter(&world).count());
    assert_eq!(0, query_with_static.iter(&world).count());

    world.mutate_entity(*entities.get(1).unwrap(), |tags, _| {
        tags.set_tag(Arc::new(Static));
    });

    assert_eq!(3, query_without_static.iter(&world).count());
    assert_eq!(1, query_with_static.iter(&world).count());
}

#[test]
fn mutate_remove_tag() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Model(5), Static).as_tags();
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.insert_from(shared, components).to_vec();

    let query_without_static = <(Read<Pos>, Read<Rot>)>::query().filter(!tag::<Static>());
    let query_with_static = <(Read<Pos>, Read<Rot>, Tagged<Static>)>::query();

    assert_eq!(0, query_without_static.iter(&world).count());
    assert_eq!(3, query_with_static.iter(&world).count());

    world.mutate_entity(*entities.get(1).unwrap(), |tags, _| {
        tags.remove_tag::<Static>();
    });

    assert_eq!(1, query_without_static.iter(&world).count());
    assert_eq!(2, query_with_static.iter(&world).count());
}

#[test]
fn mutate_change_tag() {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let shared = (Model(5),).as_tags();
    let components = vec![
        (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
    ];

    let entities = world.insert_from(shared, components).to_vec();

    let query_model_3 = <(Read<Pos>, Read<Rot>)>::query().filter(tag_value(&Model(3)));
    let query_model_5 = <(Read<Pos>, Read<Rot>)>::query().filter(tag_value(&Model(5)));

    assert_eq!(3, query_model_5.iter(&world).count());
    assert_eq!(0, query_model_3.iter(&world).count());

    world.mutate_entity(*entities.get(1).unwrap(), |tags, _| {
        tags.set_tag(Arc::new(Model(3)));
    });

    assert_eq!(2, query_model_5.iter(&world).count());
    assert_eq!(1, query_model_3.iter(&world).count());
}
