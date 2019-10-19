use crate::{
    borrow::{Exclusive, RefMut},
    command::CommandBuffer,
    storage::ComponentTypeId,
    world::World,
};
use bit_set::BitSet;
use itertools::izip;
use rayon::prelude::*;
use std::iter::repeat;
use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

/// Stages represent discrete steps of a game's loop, such as "start", "update", "draw", "end", etc.
/// Stages have a defined execution order.
///
/// Systems run within a stage, and commit any buffered changes to the ecs at the end of a stage
/// (which may or may not be the stage within which they run, but cannot be an earlier stage).
trait Stage: Copy + PartialOrd + Ord + PartialEq + Eq {}

/// Executes all systems that are to be run within a single given stage.
pub struct StageExecutor<'a> {
    systems: &'a mut [Box<dyn Schedulable>],
    pool: &'a rayon::ThreadPool,
    static_dependants: Vec<Vec<usize>>,
    dynamic_dependants: Vec<Vec<usize>>,
    static_dependancy_counts: Vec<AtomicUsize>,
    awaiting: Vec<AtomicUsize>,
}

impl<'a> StageExecutor<'a> {
    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    pub fn new(systems: &'a mut [Box<dyn Schedulable>], pool: &'a rayon::ThreadPool) -> Self {
        if systems.len() > 1 {
            let mut static_dependancy_counts = Vec::new();

            let mut static_dependants: Vec<Vec<_>> =
                repeat(Vec::new()).take(systems.len()).collect();
            let mut dynamic_dependants: Vec<Vec<_>> =
                repeat(Vec::new()).take(systems.len()).collect();

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
                pool,
                awaiting,
                static_dependants,
                dynamic_dependants,
                static_dependancy_counts,
                systems,
            }
        } else {
            Self {
                pool,
                awaiting: Vec::with_capacity(0),
                static_dependants: Vec::with_capacity(0),
                dynamic_dependants: Vec::with_capacity(0),
                static_dependancy_counts: Vec::with_capacity(0),
                systems,
            }
        }
    }

    /// Execute this stage
    /// TODO: needs better description
    pub fn execute(&mut self, world: &World) {
        log::trace!("execute");
        self.pool.scope(|_scope| {
            match self.systems.len() {
                1 => {
                    log::trace!("Single system, just run it");
                    self.systems[0].run(world);
                }
                _ => {
                    log::trace!("Begin pool execution");
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
                            self.run_recursive(i, world);
                        });
                }
            }
        })
    }

    /// Recursively execute through the generated depedency cascade and exhaust it.
    fn run_recursive(&self, i: usize, world: &World) {
        log::trace!("run_recursive");
        self.systems[i].run(world);

        // notify dependants of the completion of this dependancy
        // execute all systems that became available upon the completion of this system
        self.static_dependants[i]
            .par_iter()
            .filter(|dep| {
                let fetch = self.awaiting[**dep].fetch_sub(1, Ordering::SeqCst);
                fetch.saturating_sub(1) == 0
            })
            .for_each(|dep| {
                // Flip the waiting bit on this system to max, so it doesnt run again this stage.
                self.awaiting[*dep].store(std::usize::MAX, Ordering::SeqCst);
                self.run_recursive(*dep, world);
            });
    }
}

/// Trait describing a schedulable type. This is implemented by `System`
pub trait Schedulable: Runnable + Send + Sync {}
impl<T> Schedulable for T where T: Runnable + Send + Sync {}

/// Trait describing a schedulable type. This is implemented by `System`
pub trait Runnable {
    fn name(&self) -> &str;
    fn explicit_dependencies(&self) -> &[String];
    fn reads(&self) -> (&[TypeId], &[ComponentTypeId]);
    fn writes(&self) -> (&[TypeId], &[ComponentTypeId]);
    fn prepare(&mut self, world: &World);
    fn accesses_archetypes(&self) -> &BitSet;
    fn run(&self, world: &World);
    fn dispose(self: Box<Self>, world: &mut World);
    fn command_buffer_mut(&self) -> RefMut<Exclusive, CommandBuffer>;
}
