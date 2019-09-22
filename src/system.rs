use crate::borrow::AtomicRefCell;
use crate::storage::ComponentTypeId;
use crate::world::World;
use bit_set::BitSet;
use itertools::izip;
use rayon::prelude::*;
use std::any::Any;
use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::repeat;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Stages represent discrete steps of a game's loop, such as "start", "update", "draw", "end", etc.
/// Stages have a defined execution order.
///
/// Systems run within a stage, and commit any buffered changes to the ecs at the end of a stage
/// (which may or may not be the stage within which they run, but cannot be an earlier stage).
trait Stage: Copy + PartialOrd + Ord + PartialEq + Eq {}

/// Executes all systems that are to be run within a single given stage.
struct StageExecutor {
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
            for res in &read_res {
                if let Some(n) = resource_last_mutated.get(res) {
                    dependancies.insert(*n);
                }
            }
            for res in &write_res {
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
            for comp in &read_comp {
                if let Some(ns) = component_mutated.get(comp) {
                    for n in ns {
                        comp_dependancies.insert(*n);
                    }
                }
            }
            for comp in &write_comp {
                if let Some(ns) = component_mutated.get(comp) {
                    for n in ns {
                        comp_dependancies.insert(*n);
                    }
                }
                component_mutated.entry(*comp).or_insert(Vec::new()).push(i);
            }
            for dep in comp_dependancies {
                dynamic_dependants[dep].push(i);
            }
        }

        Self {
            awaiting: Vec::new(),
            static_dependants,
            dynamic_dependants,
            static_dependancy_counts,
            systems,
        }
    }

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

    fn run_recursive(&self, i: usize, resources: &Resources, world: &World) {
        self.systems[i].run(resources, world);

        // notify dependants of the completion of this dependancy
        // execute all systems that became available upon the completion of this system
        self.static_dependants[i]
            .par_iter()
            .filter(|dep| self.awaiting[**dep].fetch_sub(1, Ordering::SeqCst) == 0)
            .for_each(|dep| self.run_recursive(*dep, resources, world));
    }
}

trait Schedulable: Sync + Send {
    fn reads(&self) -> (Vec<TypeId>, Vec<ComponentTypeId>);
    fn writes(&self) -> (Vec<TypeId>, Vec<ComponentTypeId>);
    fn prepare(&mut self, world: &World);
    fn accesses_archetypes(&self) -> &BitSet;
    fn run(&self, resources: &Resources, world: &World);
}

trait Resource: Any + Send + Sync {}

struct Resources(HashMap<TypeId, AtomicRefCell<Box<dyn Resource>>>);

// implement Accessor for tupes of Read/Write<Resource>

trait Accessor: Send + Sync {
    type Output;

    fn reads() -> Vec<TypeId>;
    fn writes() -> Vec<TypeId>;
    fn fetch(resources: &Resources) -> Self::Output;
}

impl Accessor for () {
    type Output = ();

    fn reads() -> Vec<TypeId> { Vec::default() }
    fn writes() -> Vec<TypeId> { Vec::default() }
    fn fetch(_: &Resources) -> () {}
}

// * implement QuerySet for tuples of queries
// * likely actually wrapped in another struct, to cache the archetype sets for each query
// * prepared queries will each re-use the archetype set results in their iterators so
// that the archetype filters don't need to be run again - can also cache this between runs
// and only append new archetype matches each frame
// * per-query archetype matches stored as simple Vec<usize> - filter_archetypes() updates them and writes
// the union of all queries into the BitSet provided, to be used to schedule the system as a whole

trait QuerySet<'a>: Send + Sync {
    type PreparedQueries: 'a;

    fn reads() -> Vec<ComponentTypeId>;
    fn writes() -> Vec<ComponentTypeId>;
    fn filter_archetypes(&mut self, world: &World, archetypes: &mut BitSet);
    fn prepare(&'a self, world: &'a World) -> Self::PreparedQueries;
}

impl<'a> QuerySet<'a> for () {
    type PreparedQueries = ();

    fn reads() -> Vec<ComponentTypeId> { Vec::default() }
    fn writes() -> Vec<ComponentTypeId> { Vec::default() }
    fn filter_archetypes(&mut self, _: &World, _: &mut BitSet) {}
    fn prepare(&'a self, _: &'a World) -> Self::PreparedQueries { () }
}

struct System<
    R: Accessor,
    Q: for<'a> QuerySet<'a>,
    F: for<'a> Fn(R::Output, <Q as QuerySet<'a>>::PreparedQueries),
> {
    _resources: PhantomData<R>,
    queries: Q,
    run_fn: F,
    archetypes: BitSet,
}

impl<R, Q, F> Schedulable for System<R, Q, F>
where
    R: Accessor,
    Q: for<'a> QuerySet<'a>,
    F: for<'a> Fn(R::Output, <Q as QuerySet<'a>>::PreparedQueries) + Send + Sync,
{
    fn reads(&self) -> (Vec<TypeId>, Vec<ComponentTypeId>) { (R::reads(), Q::reads()) }

    fn writes(&self) -> (Vec<TypeId>, Vec<ComponentTypeId>) { (R::writes(), Q::writes()) }

    fn prepare(&mut self, world: &World) {
        self.queries.filter_archetypes(world, &mut self.archetypes);
    }

    fn accesses_archetypes(&self) -> &BitSet { &self.archetypes }

    fn run(&self, resources: &Resources, world: &World) {
        let resources = R::fetch(resources);
        let queries = self.queries.prepare(world);

        (self.run_fn)(resources, queries);
    }
}
