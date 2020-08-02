# Legion

[![Build Status][build_img]][build_lnk] [![Crates.io][crates_img]][crates_lnk] [![Docs.rs][doc_img]][doc_lnk]

[build_img]: https://github.com/TomGillen/legion/workflows/CI/badge.svg
[build_lnk]: https://github.com/TomGillen/legion/actions
[crates_img]: https://img.shields.io/crates/v/legion.svg
[crates_lnk]: https://crates.io/crates/legion
[doc_img]: https://docs.rs/legion/badge.svg
[doc_lnk]: https://docs.rs/legion

Legion aims to be a feature rich high performance ECS library for Rust game projects with minimal boilerplate.

## Getting Started

### Worlds

[Worlds](https://docs.rs/legion/latest/legion/world/struct.World.html) are collections of [entities](https://docs.rs/legion/latest/legion/entity/struct.Entity.html), where each entity can have an arbitrary collection of [components](https://docs.rs/legion/latest/legion/storage/component/trait.Component.html) attached.

```rust
use legion::*;
let world = World::new();
```

Entities can be inserted via either `push` (for a single entity) or `extend` (for a collection of entities with the same component types). The world will create a unique ID for each entity upon insertion that you can use to refer to that entity later.

```rust
// a component is any type that is 'static, sized, send and sync
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    x: f32,
    y: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Velocity {
    dx: f32,
    dy: f32,
}

// push a component tuple into the world to create an entity
let entity: Entity = world.push((Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }));

// or extend via an IntoIterator of tuples to add many at once (this is faster)
let entities: &[Entity] = world.extend(vec![
    (Position { x: 0.0, y: 0.0 }, Velocity { dx: 0.0, dy: 0.0 }),
    (Position { x: 1.0, y: 1.0 }, Velocity { dx: 0.0, dy: 0.0 }),
    (Position { x: 2.0, y: 2.0 }, Velocity { dx: 0.0, dy: 0.0 }),
]);
```

You can access entities via [entries](https://docs.rs/legion/latest/legion/entry/index.html). Entries allow you to query an entity to find out what types of components are attached to it, to get component references, or to add and remove components.

```rust
// entries return `None` if the entity does not exist
if let Some(mut entry) = world.entry(entity) {
    // access information about the entity's archetype
    println!("{:?} has {:?}", entity, entry.archetype().layout().component_types());

    // add an extra component
    entry.add_component(12f32);

    // access the entity's components, returns `None` if the entity does not have the component
    assert_eq!(entry.get_component::<f32>().unwrap(), &12f32);
}
```

### Queries

Entries are not the most convenient or performant way to search or bulk-access a world. [Queries](https://docs.rs/legion/latest/legion/query/index.html) allow for high performance and expressive iteration through the entities in a world.

```rust
// you define a query be declaring what components you want to find, and how you will access them
let mut query = <&Position>::query();

// you can then iterate through the components found in the world
for position in query.iter(&world) {
    println!("{:?}", position);
}
```

You can search for entities which have all of a set of components.

```rust
// construct a query from a "view tuple"
let mut query = <(&Velocity, &mut Position)>::query();

// this time we have &Velocity and &mut Position
for (velocity, position) in query.iter_mut(&mut world) {
    position.x += velocity.x;
    position.y += velocity.y;
}
```

You can augment a basic query with additional filters. For example, you can choose to exclude entities which also have a certain component, or only include entities for which a certain component has changed since the last time the query ran (this filtering is conservative and course-grained)

```rust
// you can use boolean expressions when adding filters
let mut query = <(&Velocity, &mut Position)>::query()
    .filter(!component::<Ignore>() & maybe_changed::<Position>());

for (velocity, position) in query.iter_mut(&mut world) {
    position.x += velocity.dx;
    position.y += velocity.dy;
}
```

There is much more than can be done with queries. See the [module documentation](https://docs.rs/legion/latest/legion/query/index.html) for more information.

### Systems

You may have noticed that when we wanted to write to a component, we needed to use `iter_mut` to iterate through our query. But perhaps your application wants to be able to process different components on different entities, perhaps even at the same time in parallel? While it is possible to do this manually (see [World](https://docs.rs/legion/latest/legion/world/struct.World.html)::split), this is very difficult to do when the different pieces of the application don't know what components each other need, or might or might not even have conflicting access requirements.

[Systems](https://docs.rs/legion/latest/legion/systems/system/index.html) and the [Schedule](https://docs.rs/legion/latest/legion/systems/schedule/struct.Schedule.html) automates this process, and can even schedule work at a more granular level than you can otherwise do manually. A system is a unit of work. Each system is defined as a function which is provided access to queries and shared [resources](https://docs.rs/legion/latest/legion/systems/resources/struct.Resources.l). These systems can then be appended to a schedule, which is a linear sequence of systems, ordered by when side effects (such as writes to components) should be observed. The schedule will automatically parallelize the execution of all systems whilst maintaining the apparent order of execution from the perspective of each system.

```rust
// a system fn which loops through Position and Velocity components, and reads the Time shared resource
// the #[system] macro generates a fn called update_positions_system() which will construct our system
#[system(for_each)]
fn update_positions(pos: &mut Position, vel: &Velocity, #[resource] time: &Time) {
    pos.x += vel.dx * time.elapsed_seconds;
    pos.y += vel.dy * time.elapsed_seconds;
}

// construct a schedule (you should do this on init)
let mut schedule = Schedule::builder()
    .add_system(update_positions_system())
    .build();

// run our schedule (you should do this each update)
schedule.execute(&mut world, &mut resources);
```

See the [systems module](https://docs.rs/legion/latest/legion/systems/index.html) and the [system proc macro](https://docs.rs/legion/latest/legion/attr.system.html) for more information.

## Feature Flags

Legion provides a few feature flags:  

* `par-iter` - Enables parallel iterators and parallel schedule execution via the rayon library. Enabled by default.
* `extended-tuple-impls` - Extends the maximum size of view and component tuples from 8 to 24, at the cost of increased compile times. Off by default.
* `serialize` - Enables the serde serialization module and associated functionality. Enabled by default.
* `crossbeam-events` - Implements the `EventSender` trait for crossbeam `Sender` channels, allowing them to be used for event subscriptions. Enabled by default.

## WASM

Legion runs with parallelism on by default, which is not currently supported by Web Assembly as it runs single-threaded. Therefore, to build for WASM, ensure you set `default-features = false` in Cargo.toml. Additionally, you must enable either the `stdweb` or `wasm-bindgen` features, which will be proxied through to the `uuid` crate. See the [uuid crate](https://github.com/uuid-rs/uuid#dependencies) for more information.

```toml
legion = { version = "*", default-features = false, features = ["stdweb"] }
```
