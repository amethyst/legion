use crate::{
    borrow::{Exclusive, RefMut},
    command::CommandBuffer,
    resource::ResourceTypeId,
    storage::ComponentTypeId,
    world::World,
};
use bit_set::BitSet;
use itertools::izip;
use std::iter::repeat;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(feature = "par-iter")]
use rayon::prelude::*;

/// Empty trait which defines a `System` as schedulable by the dispatcher - this requires that the
/// type is both `Send` and `Sync`.
///
/// This is automatically implemented for all types that implement `Runnable` which meet the requirements.
pub trait Schedulable: Runnable + Send + Sync {}
impl<T> Schedulable for T where T: Runnable + Send + Sync {}

/// Describes which archetypes a system declares access to.
pub enum ArchetypeAccess {
    /// All archetypes.
    All,
    /// Some archetypes.
    Some(BitSet),
}

impl ArchetypeAccess {
    pub fn is_disjoint(&self, other: &ArchetypeAccess) -> bool {
        match self {
            Self::All => false,
            Self::Some(mine) => match other {
                Self::All => false,
                Self::Some(theirs) => mine.is_disjoint(theirs),
            },
        }
    }
}

/// Trait describing a schedulable type. This is implemented by `System`
pub trait Runnable {
    fn name(&self) -> &str;
    fn reads(&self) -> (&[ResourceTypeId], &[ComponentTypeId]);
    fn writes(&self) -> (&[ResourceTypeId], &[ComponentTypeId]);
    fn prepare(&mut self, world: &World);
    fn accesses_archetypes(&self) -> &ArchetypeAccess;
    fn run(&self, world: &World);
    fn dispose(self: Box<Self>, world: &mut World);
    fn command_buffer_mut(&self) -> RefMut<Exclusive, CommandBuffer>;
}

/// Stages represent discrete steps of a game's loop, such as "start", "update", "draw", "end", etc.
/// Stages have a defined execution order.
///
/// Systems run within a stage, and commit any buffered changes to the ecs at the end of a stage
/// (which may or may not be the stage within which they run, but cannot be an earlier stage).
trait Stage: Copy + PartialOrd + Ord + PartialEq + Eq {}

/// Executes all systems that are to be run within a single given stage.
pub struct StageExecutor<'a> {
    systems: &'a mut [Box<dyn Schedulable>],
    #[cfg(feature = "par-iter")]
    pool: &'a rayon::ThreadPool,
    #[cfg(feature = "par-iter")]
    static_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "par-iter")]
    dynamic_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "par-iter")]
    static_dependency_counts: Vec<AtomicUsize>,
    #[cfg(feature = "par-iter")]
    awaiting: Vec<AtomicUsize>,
}

impl<'a> StageExecutor<'a> {
    #[cfg(not(feature = "par-iter"))]
    pub fn new(systems: &'a mut [Box<dyn Schedulable>]) -> Self { Self { systems } }

    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    #[cfg(feature = "par-iter")]
    #[allow(clippy::cognitive_complexity)]
    // TODO: we should break this up
    pub fn new(systems: &'a mut [Box<dyn Schedulable>], pool: &'a rayon::ThreadPool) -> Self {
        if systems.len() > 1 {
            let mut static_dependency_counts = Vec::with_capacity(systems.len());

            let mut static_dependants: Vec<Vec<_>> =
                repeat(Vec::with_capacity(64)).take(systems.len()).collect();
            let mut dynamic_dependants: Vec<Vec<_>> =
                repeat(Vec::with_capacity(64)).take(systems.len()).collect();

            let mut resource_last_mutated = HashMap::<ResourceTypeId, usize>::with_capacity(64);
            let mut resource_last_read = HashMap::<ResourceTypeId, usize>::with_capacity(64);
            let mut component_mutated = HashMap::<ComponentTypeId, Vec<usize>>::with_capacity(64);

            for (i, system) in systems.iter().enumerate() {
                log::debug!("Building dependency: {} ({})", system.name(), i);

                let (read_res, read_comp) = system.reads();
                let (write_res, write_comp) = system.writes();

                // find resource access dependencies
                let mut dependencies = HashSet::with_capacity(64);
                for res in read_res {
                    log::trace!("Read resource: {:?}", res);
                    if let Some(n) = resource_last_mutated.get(res) {
                        dependencies.insert(*n);
                    }
                    resource_last_read.insert(*res, i);
                }
                for res in write_res {
                    log::trace!("Write resource: {:?}", res);
                    // Writes have to be exclusive, so we are dependent on reads too
                    if let Some(n) = resource_last_read.get(res) {
                        log::trace!("Added dep: {:?}", n);
                        dependencies.insert(*n);
                    }

                    if let Some(n) = resource_last_mutated.get(res) {
                        log::trace!("Added dep: {:?}", n);
                        dependencies.insert(*n);
                    }

                    resource_last_mutated.insert(*res, i);
                }

                static_dependency_counts.push(AtomicUsize::from(dependencies.len()));
                log::debug!("dependencies: {:?}", dependencies);
                for dep in dependencies {
                    log::debug!("static_dependants.push: {:?}", dep);
                    static_dependants[dep].push(i);
                }

                // find component access dependencies
                let mut comp_dependencies = HashSet::new();
                for comp in read_comp {
                    if let Some(ns) = component_mutated.get(comp) {
                        for n in ns {
                            comp_dependencies.insert(*n);
                        }
                    }
                }
                for comp in write_comp {
                    if let Some(ns) = component_mutated.get(comp) {
                        for n in ns {
                            comp_dependencies.insert(*n);
                        }
                    }
                    component_mutated
                        .entry(*comp)
                        .or_insert_with(Vec::new)
                        .push(i);
                }
                log::debug!("comp_dependencies: {:?}", &comp_dependencies);
                for dep in comp_dependencies {
                    if dep != i { // dont be dependent on ourselves
                        dynamic_dependants[dep].push(i);
                    }
                }
            }

            if log::log_enabled!(log::Level::Debug) {
                log::debug!("static_dependants: {:?}", static_dependants);
                log::debug!("dynamic_dependants: {:?}", dynamic_dependants);
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
                static_dependency_counts,
                systems,
            }
        } else {
            Self {
                pool,
                awaiting: Vec::with_capacity(0),
                static_dependants: Vec::with_capacity(0),
                dynamic_dependants: Vec::with_capacity(0),
                static_dependency_counts: Vec::with_capacity(0),
                systems,
            }
        }
    }

    /// This is a linear executor which just runs the system in their given order.
    ///
    /// Only enabled with par-iter is disabled
    #[cfg(not(feature = "par-iter"))]
    pub fn execute(&mut self, world: &mut World) {
        self.systems.iter_mut().for_each(|system| {
            system.run(world);
        });

        // Flush the command buffers of all the systems
        self.systems.iter().for_each(|system| {
            system.command_buffer_mut().write(world);
        });
    }

    /// Executes this stage. Execution is recursively conducted in a draining fashion. Systems are
    /// ordered based on 1. their resource access, and then 2. their insertion order. systems are
    /// executed in the pool provided at construction, and this function does not return until all
    /// systems in this stage have completed.
    #[cfg(feature = "par-iter")]
    pub fn execute(&mut self, world: &mut World) {
        log::trace!("execute");

        rayon::join(
            || {},
            || {
                match self.systems.len() {
                    1 => {
                        log::trace!("Single system, just run it");
                        self.systems[0].run(world);
                    }
                    _ => {
                        log::trace!("Begin pool execution");
                        let systems = &mut self.systems;
                        let static_dependency_counts = &self.static_dependency_counts;
                        let awaiting = &mut self.awaiting;

                        // prepare all systems - archetype filters are pre-executed here
                        systems.par_iter_mut().for_each(|sys| sys.prepare(world));

                        // determine dynamic dependencies
                        izip!(
                            systems.iter(),
                            self.static_dependants.iter_mut(),
                            self.dynamic_dependants.iter_mut()
                        )
                        .par_bridge()
                        .for_each(|(sys, static_dep, dyn_dep)| {
                            use std::any::Any;

                            let archetypes = sys.accesses_archetypes();
                            for i in (0..dyn_dep.len()).rev() {
                                let dep = dyn_dep[i];
                                let other = &systems[dep];

                                // if the archetype sets intersect,
                                // then we can move the dynamic dependant into the static dependants set
                                if !other.accesses_archetypes().is_disjoint(archetypes) {
                                    static_dep.push(dep);
                                    dyn_dep.swap_remove(i);
                                    static_dependency_counts[dep].fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        });

                        // initialize dependency tracking
                        for (i, count) in static_dependency_counts.iter().enumerate() {
                            awaiting[i].store(count.load(Ordering::Relaxed), Ordering::Relaxed);
                        }

                        log::trace!("Initialized awaiting: {:?}", awaiting);

                        let awaiting = &self.awaiting;

                        // execute all systems with no outstanding dependencies
                        (0..systems.len())
                            .filter(|i| awaiting[*i].load(Ordering::SeqCst) == 0)
                            .for_each(|i| {
                                self.run_recursive(i, world);
                            });
                    }
                }
            },
        );

        // Flush the command buffers of all the systems
        self.systems.iter().for_each(|system| {
            system.command_buffer_mut().write(world);
        });
    }

    /// Recursively execute through the generated depedency cascade and exhaust it.
    #[cfg(feature = "par-iter")]
    fn run_recursive(&self, i: usize, world: &World) {
        log::trace!("run_recursive: {} ({})", self.systems[i].name(), i);
        self.systems[i].run(world);

        self.static_dependants[i].par_iter().for_each(|dep| {
            match self.awaiting[*dep].compare_exchange(
                1,
                std::usize::MAX,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.run_recursive(*dep, world);
                }
                Err(_) => {
                    self.awaiting[*dep].fetch_sub(1, Ordering::Relaxed);
                }
            }
        });
    }
}
