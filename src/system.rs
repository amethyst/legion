use crate::borrow::AtomicRefCell;
use crate::command::CommandBuffer;
use crate::cons::{ConsAppend, ConsFlatten};
use crate::filter::EntityFilter;
use crate::query::{Chunk, ChunkDataIter, ChunkEntityIter, ChunkViewIter, Query, View};
use crate::resource::{Accessor, Resource, ResourceAccessType, Resources};
use crate::storage::{ComponentTypeId, TagTypeId};
use crate::world::World;
use bit_set::BitSet;
use derivative::Derivative;
use itertools::izip;
use rayon::prelude::*;
use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::repeat;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Stages represent discrete steps of a game's loop, such as "start", "update", "draw", "end", etc.
/// Stages have a defined execution order.
///
/// Systems run within a stage, and commit any buffered changes to the ecs at the end of a stage
/// (which may or may not be the stage within which they run, but cannot be an earlier stage).
trait Stage: Copy + PartialOrd + Ord + PartialEq + Eq {}

/// Executes all systems that are to be run within a single given stage.
pub struct StageExecutor {
    systems: Vec<Box<dyn Schedulable>>,
    static_dependants: Vec<Vec<usize>>,
    dynamic_dependants: Vec<Vec<usize>>,
    static_dependancy_counts: Vec<AtomicUsize>,
    awaiting: Vec<AtomicUsize>,
}

impl StageExecutor {
    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    pub fn new<I: IntoIterator<Item = Box<dyn Schedulable>>>(systems: I) -> Self {
        let systems: Vec<_> = systems.into_iter().collect();
        let mut static_dependants: Vec<Vec<_>> = repeat(Vec::new()).take(systems.len()).collect();
        let mut dynamic_dependants: Vec<Vec<_>> = repeat(Vec::new()).take(systems.len()).collect();
        let mut static_dependancy_counts = Vec::new();

        let mut resource_last_mutated = HashMap::<TypeId, usize>::new();
        let mut component_mutated = HashMap::<ComponentTypeId, Vec<usize>>::new();

        for (i, system) in systems.iter().enumerate() {
            let (read_res, read_comp) = system.reads();
            let (write_res, write_comp) = system.writes();

            // find resource access dependancies
            let mut dependancies = HashSet::new();
            for res in read_res {
                if let Some(n) = resource_last_mutated.get(res) {
                    dependancies.insert(*n);
                }
            }
            for res in write_res {
                if let Some(n) = resource_last_mutated.get(res) {
                    dependancies.insert(*n);
                }
                resource_last_mutated.insert(*res, i);
            }
            static_dependancy_counts.push(AtomicUsize::from(dependancies.len()));
            for dep in dependancies {
                static_dependants[dep].push(i);
            }

            // find component access dependancies
            let mut comp_dependancies = HashSet::new();
            for comp in read_comp {
                if let Some(ns) = component_mutated.get(comp) {
                    for n in ns {
                        comp_dependancies.insert(*n);
                    }
                }
            }
            for comp in write_comp {
                if let Some(ns) = component_mutated.get(comp) {
                    for n in ns {
                        comp_dependancies.insert(*n);
                    }
                }
                component_mutated
                    .entry(*comp)
                    .or_insert_with(Vec::new)
                    .push(i);
            }
            for dep in comp_dependancies {
                dynamic_dependants[dep].push(i);
            }
        }

        let mut awaiting = Vec::with_capacity(systems.len());
        systems
            .iter()
            .for_each(|_| awaiting.push(AtomicUsize::new(0)));

        Self {
            awaiting,
            static_dependants,
            dynamic_dependants,
            static_dependancy_counts,
            systems,
        }
    }

    /// Execute this stage
    /// TODO: needs better description
    pub fn execute(&mut self, resources: &Resources, world: &World) {
        let systems = &mut self.systems;
        let static_dependancy_counts = &self.static_dependancy_counts;
        let awaiting = &mut self.awaiting;

        // prepare all systems - archetype filters are pre-executed here
        systems.par_iter_mut().for_each(|sys| sys.prepare(world));

        // determine dynamic dependancies
        izip!(
            systems.iter(),
            self.static_dependants.iter_mut(),
            self.dynamic_dependants.iter_mut()
        )
        .par_bridge()
        .for_each(|(sys, static_dep, dyn_dep)| {
            let archetypes = sys.accesses_archetypes();
            for i in (0..dyn_dep.len()).rev() {
                let dep = dyn_dep[i];
                let other = &systems[dep];

                // if the archetype sets intersect,
                // then we can move the dynamic dependant into the static dependants set
                if !other.accesses_archetypes().is_disjoint(archetypes) {
                    static_dep.push(dep);
                    dyn_dep.swap_remove(i);
                    static_dependancy_counts[dep].fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        // initialize dependancy tracking
        for (i, count) in static_dependancy_counts.iter().enumerate() {
            awaiting[i].store(count.load(Ordering::SeqCst), Ordering::SeqCst);
        }

        let awaiting = &self.awaiting;

        // execute all systems with no outstanding dependancies
        (0..systems.len())
            .into_par_iter()
            .filter(|i| awaiting[*i].load(Ordering::SeqCst) == 0)
            .for_each(|i| {
                self.run_recursive(i, resources, world);
            });
    }

    /// Recursively execute through the generated depedency cascade and exhaust it.
    fn run_recursive(&self, i: usize, resources: &Resources, world: &World) {
        self.systems[i].run(resources, world);

        // notify dependants of the completion of this dependancy
        // execute all systems that became available upon the completion of this system
        self.static_dependants[i]
            .par_iter()
            .filter(|dep| {
                let fetch = self.awaiting[**dep].fetch_sub(1, Ordering::SeqCst);
                fetch - 1 == 0
            })
            .for_each(|dep| self.run_recursive(*dep, resources, world));
    }
}

/// Trait describing a schedulable type. This is implemented by `System`
pub trait Schedulable: Sync + Send {
    fn reads(&self) -> (&[TypeId], &[ComponentTypeId]);
    fn writes(&self) -> (&[TypeId], &[ComponentTypeId]);
    fn prepare(&mut self, world: &World);
    fn accesses_archetypes(&self) -> &BitSet;
    fn run(&self, resources: &Resources, world: &World);
}

/// Structure used by `SystemAccess` for describing access to the provided `T`
#[derive(Derivative, Debug, Clone)]
#[derivative(Default(bound = ""))]
pub struct Access<T> {
    reads: Vec<T>,
    writes: Vec<T>,
}

/// Structure describing the resource and component access conditions of the system.
#[derive(Derivative, Debug, Clone)]
#[derivative(Default(bound = ""))]
pub struct SystemAccess {
    pub resources: Access<TypeId>,
    pub components: Access<ComponentTypeId>,
    pub tags: Access<TagTypeId>,
}

/// * implement QuerySet for tuples of queries
/// * likely actually wrapped in another struct, to cache the archetype sets for each query
/// * prepared queries will each re-use the archetype set results in their iterators so
/// that the archetype filters don't need to be run again - can also cache this between runs
/// and only append new archetype matches each frame
/// * per-query archetype matches stored as simple Vec<usize> - filter_archetypes() updates them and writes
/// the union of all queries into the BitSet provided, to be used to schedule the system as a whole
///
/// FIXME: This would have an associated lifetime and would hold references instead of pointers,
/// but this is a workaround for lack of GATs and bugs around HRTBs combined with associated types.
/// See https://github.com/rust-lang/rust/issues/62529
pub struct PreparedQuery<V, F>
where
    V: for<'v> View<'v>,
    F: EntityFilter,
{
    world: *const World,
    query: *mut Query<V, F>,
}

impl<V, F> PreparedQuery<V, F>
where
    V: for<'v> View<'v>,
    F: EntityFilter,
{
    /// Safety: input references might not outlive a created instance of `PreparedQuery`.
    unsafe fn new(world: &World, query: &mut Query<V, F>) -> Self {
        Self {
            world: world as *const World,
            query: query as *mut Query<V, F>,
        }
    }

    // These methods are not unsafe, because we guarantee that `PreparedQuery` lifetime is never actually
    // in user's hands and access to internal pointers is impossible. There is no way to move the object out
    // of mutable reference through public API, because there is no way to get access to more than a single instance at a time.
    // The unsafety is an implementation detail. It can be fully safe once GATs are in the language.
    /// Gets an iterator which iterates through all chunks that match the query.
    pub fn iter_chunks<'a, 'b>(
        &'b mut self,
    ) -> ChunkViewIter<'a, 'b, V, F::ArchetypeFilter, F::ChunksetFilter, F::ChunkFilter> {
        unsafe { (&mut *self.query).iter_chunks(&*self.world) }
    }

    /// Gets an iterator which iterates through all entity data that matches the query, and also yields the the `Entity` IDs.
    pub fn iter_entities<'a, 'b>(
        &'b mut self,
    ) -> ChunkEntityIter<
        'a,
        V,
        ChunkViewIter<'a, 'b, V, F::ArchetypeFilter, F::ChunksetFilter, F::ChunkFilter>,
    > {
        unsafe { (&mut *self.query).iter_entities(&*self.world) }
    }

    /// Gets an iterator which iterates through all entity data that matches the query.
    pub fn iter<'a, 'data>(
        &'a mut self,
    ) -> ChunkDataIter<
        'data,
        V,
        ChunkViewIter<'data, 'a, V, F::ArchetypeFilter, F::ChunksetFilter, F::ChunkFilter>,
    > {
        unsafe { (&mut *self.query).iter(&*self.world) }
    }

    /// Iterates through all entity data that matches the query.
    pub fn for_each<'a, 'data, T>(&'a mut self, mut f: T)
    where
        T: Fn(<<V as View<'data>>::Iter as Iterator>::Item),
    {
        self.iter().for_each(&mut f);
    }

    /// Iterates through all entity data that matches the query in parallel.
    #[cfg(feature = "par-iter")]
    pub fn par_for_each<'a, T>(&'a mut self, f: T)
    where
        T: Fn(<<V as View<'a>>::Iter as Iterator>::Item) + Send + Sync,
    {
        self.par_iter_chunks().for_each(|mut chunk| {
            for data in chunk.iter() {
                f(data);
            }
        });
    }

    /// Gets a parallel iterator of chunks that match the query.
    #[cfg(feature = "par-iter")]
    pub fn par_iter_chunks(&mut self) -> impl ParallelIterator<Item = Chunk<'_, V>> {
        self.iter_chunks().par_bridge()
    }
}

pub trait QuerySet: Send + Sync {
    type PreparedQueries;
    fn filter_archetypes(&mut self, world: &World, archetypes: &mut BitSet);
    /// Safety: prepare call doesn't respect lifetimes of `self` and `world.
    /// The returned value cannot outlive them.
    unsafe fn prepare(&mut self, world: &World) -> Self::PreparedQueries;
    // fn unprepare(prepared: Self::PreparedQueries) -> Self;
}

macro_rules! impl_queryset_tuple {
    ($($ty: ident),*) => {
        paste::item! {
            #[allow(unused_parens, non_snake_case)]
            impl<$([<$ty V>], [<$ty F>], )*> QuerySet for ($(Query<[<$ty V>], [<$ty F>]>, )*)
            where
                $([<$ty V>]: for<'v> View<'v>,)*
                $([<$ty F>]: EntityFilter + Send + Sync,)*
            {
                type PreparedQueries = ( $(PreparedQuery<[<$ty V>], [<$ty F>]>, )*  );
                fn filter_archetypes(&mut self, world: &World, bitset: &mut BitSet) {
                    let ($($ty,)*) = self;

                    $(
                        let storage = world.storage();
                        $ty.filter.iter_archetype_indexes(storage).for_each(|id| { bitset.insert(id); });
                    )*
                }
                unsafe fn prepare(&mut self, world: &World) -> Self::PreparedQueries {
                    let ($($ty,)*) = self;
                    ($(PreparedQuery::<[<$ty V>], [<$ty F>]>::new(world, $ty),)*)
                }
            }
        }
    };
}

impl<AV, AF> QuerySet for Query<AV, AF>
where
    AV: for<'v> View<'v>,
    AF: EntityFilter + Send + Sync,
{
    type PreparedQueries = PreparedQuery<AV, AF>;
    fn filter_archetypes(&mut self, world: &World, bitset: &mut BitSet) {
        let storage = world.storage();
        self.filter.iter_archetype_indexes(storage).for_each(|id| {
            bitset.insert(id);
        });
    }
    unsafe fn prepare(&mut self, world: &World) -> Self::PreparedQueries {
        PreparedQuery::<AV, AF>::new(world, self)
    }
}

impl_queryset_tuple!(A);
impl_queryset_tuple!(A, B);
impl_queryset_tuple!(A, B, C);
impl_queryset_tuple!(A, B, C, D);
impl_queryset_tuple!(A, B, C, D, E);
impl_queryset_tuple!(A, B, C, D, E, F);
impl_queryset_tuple!(A, B, C, D, E, F, G);

/// The concrete type which contains the system closure provided by the user.  This struct should
/// not be instantiated directly, and instead should be created using `SystemBuilder`.
///
/// Implements `Schedulable` which is consumable by the `StageExecutor`, executing the closure.
///
/// Also handles caching of archetype information in a `BitSet`, as well as maintaining the provided
/// information about what queries this system will run and, as a result, its data access.
///
/// Queries are stored generically within this struct, and the `PreparedQuery` types are generated
/// on each `run` call, wrapping the world and providing the set to the user in their closure.
pub struct System<R, Q, F>
where
    R: Accessor,
    Q: QuerySet,
    F: Fn(&mut CommandBuffer, R::Output, &mut <Q as QuerySet>::PreparedQueries)
        + Send
        + Sync
        + 'static,
{
    resources: R,
    queries: AtomicRefCell<Q>,
    run_fn: F,
    archetypes: BitSet,

    // These are stored statically instead of always iterated and created from the
    // query types, which would make allocations every single request
    access: SystemAccess,

    // We pre-allocate a commnad buffer for ourself. Writes are self-draining so we never have to rellocate.
    command_buffer: AtomicRefCell<CommandBuffer>,
}

impl<R, Q, F> Schedulable for System<R, Q, F>
where
    R: Accessor,
    Q: QuerySet,
    F: Fn(&mut CommandBuffer, R::Output, &mut <Q as QuerySet>::PreparedQueries)
        + Send
        + Sync
        + 'static,
{
    fn reads(&self) -> (&[TypeId], &[ComponentTypeId]) {
        (&self.access.resources.reads, &self.access.components.reads)
    }
    fn writes(&self) -> (&[TypeId], &[ComponentTypeId]) {
        (&self.access.resources.reads, &self.access.components.reads)
    }

    fn prepare(&mut self, world: &World) {
        self.queries
            .get_mut()
            .filter_archetypes(world, &mut self.archetypes);
    }

    fn accesses_archetypes(&self) -> &BitSet { &self.archetypes }

    fn run(&self, resources: &Resources, world: &World) {
        let resources = R::fetch(resources);
        let mut queries = self.queries.get_mut();
        let mut prepared_queries = unsafe { queries.prepare(world) };

        // Give the command buffer a new entity block.
        // This should usually just pull a free block, or allocate a new one...
        // TODO: The BlockAllocator should *ensure* keeping at least 1 free block so this prevents an allocation

        (self.run_fn)(
            &mut self.command_buffer.get_mut(),
            resources,
            &mut prepared_queries,
        );
    }
}

// This builder uses a Cons/Hlist implemented in cons.rs to generated the static query types
// for this system. Access types are instead stored and abstracted in the top level vec here
// so the underlying Accessor type functions from the queries don't need to allocate.
// Otherwise, this leads to excessive alloaction for every call to reads/writes
/// The core builder of `System` types, which are systems within Legion. Systems are implemented
/// as singular closures for a given system - providing queries which should be cached for that
/// system, as well as resource access and other metadata.
/// ```rust
/// # use legion::prelude::*;
/// # use legion::resource::ResourceAccessType;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// #[derive(Copy, Clone, Debug, PartialEq)]
/// struct Static;
///
///  let mut system_one = SystemBuilder::<()>::new("TestSystem")
///            .with_resource::<TestResource>(ResourceAccessType::Read)
///            .with_query(<(Read<Position>, Tagged<Model>)>::query()
///                         .filter(!tag::<Static>() | changed::<Position>()))
///            .build(move |_resource, queries| {
///                println!("Hello world");
///               let mut count = 0;
///                {
///                    for (entity, pos) in queries.iter_entities() {
///                        assert_eq!(expected.get(&entity).unwrap().0, *pos);
///                        count += 1;
///                    }
///                }
///
///                assert_eq!(components.len(), count);
///            });
/// ```
pub struct SystemBuilder<Q = (), R = ()> {
    name: String,

    queries: Q,
    resources: R,

    resource_access: Access<TypeId>,
    component_access: Access<ComponentTypeId>,
}

impl<Q, R> SystemBuilder<Q, R>
where
    Q: 'static + Send + ConsFlatten,
    R: 'static + Send + ConsFlatten,
{
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: &str) -> SystemBuilder {
        SystemBuilder {
            name: name.to_string(),
            queries: (),
            resources: (),
            resource_access: Access::default(),
            component_access: Access::default(),
        }
    }

    pub fn with_query<V, F>(
        mut self,
        query: Query<V, F>,
    ) -> SystemBuilder<<Q as ConsAppend<Query<V, F>>>::Output, R>
    where
        V: for<'a> View<'a>,
        F: 'static + EntityFilter,
        Q: ConsAppend<Query<V, F>>,
    {
        self.component_access.reads.extend(V::read_types().iter());
        self.component_access.writes.extend(V::write_types().iter());

        SystemBuilder {
            name: self.name,
            queries: ConsAppend::append(self.queries, query),
            resources: self.resources,
            resource_access: self.resource_access,
            component_access: self.component_access,
        }
    }

    pub fn with_resource<T>(
        mut self,
        access_type: ResourceAccessType,
    ) -> SystemBuilder<Q, <R as ConsAppend<()>>::Output>
    where
        T: 'static + Resource,
        R: ConsAppend<()>,
        <R as ConsAppend<()>>::Output: ConsFlatten,
    {
        match access_type {
            ResourceAccessType::Read => self.resource_access.reads.push(TypeId::of::<T>()),
            ResourceAccessType::Write => self.resource_access.writes.push(TypeId::of::<T>()),
        }

        SystemBuilder {
            resources: ConsAppend::append(self.resources, ()),
            name: self.name,
            queries: self.queries,
            resource_access: self.resource_access,
            component_access: self.component_access,
        }
    }

    pub fn build<F>(self, run_fn: F) -> Box<dyn Schedulable>
    where
        <R as ConsFlatten>::Output: Accessor + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: Fn(
                &mut CommandBuffer,
                <<R as ConsFlatten>::Output as Accessor>::Output,
                &mut <<Q as ConsFlatten>::Output as QuerySet>::PreparedQueries,
            ) + Send
            + Sync
            + 'static,
    {
        Box::new(System {
            run_fn,
            resources: self.resources.flatten(),
            queries: AtomicRefCell::new(self.queries.flatten()),
            archetypes: BitSet::default(), //TODO:
            access: SystemAccess {
                resources: self.resource_access,
                components: self.component_access,
                tags: Access::default(),
            },
            command_buffer: AtomicRefCell::new(CommandBuffer::default()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use crate::resource::{ResourceAccessType, Resources};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);
    #[derive(Default)]
    struct TestResource(pub i32);

    #[test]
    fn builder_schedule_execute() {
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();
        let resources = Resources::default();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];

        let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

        for (i, e) in world.insert((), components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        #[derive(Debug, Eq, PartialEq)]
        pub enum TestSystems {
            TestSystemOne,
            TestSystemTwo,
            TestSystemThree,
        }

        let runs = Arc::new(Mutex::new(Vec::new()));

        let system_one_runs = runs.clone();
        let system_one = SystemBuilder::<()>::new("TestSystem1")
            .with_resource::<TestResource>(ResourceAccessType::Read)
            .with_query(Read::<Pos>::query())
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _resource, _queries| {
                log::trace!("TestSystem1");
                system_one_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemOne);
            });

        let system_two_runs = runs.clone();
        let system_two = SystemBuilder::<()>::new("TestSystem2")
            .with_resource::<TestResource>(ResourceAccessType::Read)
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _resource, _queries| {
                log::trace!("TestSystem2");
                system_two_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemTwo);
            });

        let order = vec![TestSystems::TestSystemOne, TestSystems::TestSystemTwo];

        let systems = vec![system_one, system_two];

        let mut executor = StageExecutor::new(systems);
        executor.execute(&resources, &world);
        assert_eq!(order, *(runs.lock().unwrap()));
    }

    #[test]
    fn builder_create_and_execute() {
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();
        let resources = Resources::default();

        let components = vec![
            (Pos(1., 2., 3.), Vel(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Vel(0.4, 0.5, 0.6)),
        ];

        let mut expected = HashMap::<Entity, (Pos, Vel)>::new();

        for (i, e) in world.insert((), components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let mut system = SystemBuilder::<()>::new("TestSystem")
            .with_resource::<TestResource>(ResourceAccessType::Read)
            .with_query(Read::<Pos>::query())
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _resource, queries| {
                let mut count = 0;
                {
                    for (entity, pos) in queries.0.iter_entities() {
                        assert_eq!(expected.get(&entity).unwrap().0, *pos);
                        count += 1;
                    }
                }

                assert_eq!(components.len(), count);
            });
        system.prepare(&world);
        system.run(&resources, &world);
    }
}
