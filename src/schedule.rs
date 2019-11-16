use crate::system::SystemId;
use crate::{
    borrow::{Exclusive, RefMut},
    command::CommandBuffer,
    resource::ResourceTypeId,
    storage::ComponentTypeId,
    world::World,
};
use bit_set::BitSet;
use itertools::izip;
use std::fmt::Debug;
use std::fmt::Display;
use std::iter::repeat;
use std::marker::PhantomData;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};
use tracing::{span, trace, Level};

#[cfg(feature = "par-schedule")]
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
    fn name(&self) -> &SystemId;
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
pub trait Stage: Copy + PartialOrd + Ord + PartialEq + Eq + Display + Debug {}

/// Executes all systems that are to be run within a single given stage.
pub struct StageExecutor {
    systems: Vec<Box<dyn Schedulable>>,
    #[cfg(feature = "par-schedule")]
    static_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "par-schedule")]
    dynamic_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "par-schedule")]
    static_dependency_counts: Vec<AtomicUsize>,
    #[cfg(feature = "par-schedule")]
    awaiting: Vec<AtomicUsize>,
}

impl StageExecutor {
    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    #[cfg(not(feature = "par-schedule"))]
    pub fn new(systems: Vec<Box<dyn Schedulable>>) -> Self { Self { systems } }

    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    #[cfg(feature = "par-schedule")]
    #[allow(clippy::cognitive_complexity)]
    // TODO: we should break this up
    pub fn new(systems: Vec<Box<dyn Schedulable>>) -> Self {
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
                let span = span!(
                    Level::TRACE,
                    "Building system dependencies",
                    system = %system.name()
                );
                let _guard = span.enter();

                let (read_res, read_comp) = system.reads();
                let (write_res, write_comp) = system.writes();

                // find resource access dependencies
                let mut dependencies = HashSet::with_capacity(64);
                for res in read_res {
                    trace!(resource = ?res, "Read resource");
                    if let Some(n) = resource_last_mutated.get(res) {
                        trace!(system_index = n, "Added write dependency");
                        dependencies.insert(*n);
                    }
                    resource_last_read.insert(*res, i);
                }
                for res in write_res {
                    trace!(resource = ?res, "Write resource");
                    // Writes have to be exclusive, so we are dependent on reads too
                    if let Some(n) = resource_last_read.get(res) {
                        trace!(system_index = n, "Added read dependency");
                        dependencies.insert(*n);
                    }

                    if let Some(n) = resource_last_mutated.get(res) {
                        trace!(system_index = n, "Added write dependency");
                        dependencies.insert(*n);
                    }

                    resource_last_mutated.insert(*res, i);
                }

                static_dependency_counts.push(AtomicUsize::from(dependencies.len()));
                trace!(dependants = ?dependencies, "Computed static dependants");
                for dep in dependencies {
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

                trace!(depentants = ?comp_dependencies, "Computed dynamic dependants");
                for dep in comp_dependencies {
                    dynamic_dependants[dep].push(i);
                }
            }

            trace!(
                ?static_dependants,
                ?dynamic_dependants,
                "Computed system dependencies"
            );

            let mut awaiting = Vec::with_capacity(systems.len());
            systems
                .iter()
                .for_each(|_| awaiting.push(AtomicUsize::new(0)));

            Self {
                awaiting,
                static_dependants,
                dynamic_dependants,
                static_dependency_counts,
                systems,
            }
        } else {
            Self {
                awaiting: Vec::with_capacity(0),
                static_dependants: Vec::with_capacity(0),
                dynamic_dependants: Vec::with_capacity(0),
                static_dependency_counts: Vec::with_capacity(0),
                systems,
            }
        }
    }

    /// Converts this executor into a vector of its component systems.
    pub fn into_vec(self) -> Vec<Box<dyn Schedulable>> { self.systems }

    /// This is a linear executor which just runs the system in their given order.
    ///
    /// Only enabled with par-schedule is disabled
    #[cfg(not(feature = "par-schedule"))]
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
    ///
    /// Call from within `rayon::ThreadPool::install()` to execute within a specific thread pool.
    #[cfg(feature = "par-schedule")]
    pub fn execute(&mut self, world: &mut World) {
        rayon::join(
            || {},
            || {
                match self.systems.len() {
                    1 => {
                        self.systems[0].run(world);
                    }
                    _ => {
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

                        let awaiting = &self.awaiting;

                        // execute all systems with no outstanding dependencies
                        (0..systems.len())
                            .into_par_iter()
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
    #[cfg(feature = "par-schedule")]
    fn run_recursive(&self, i: usize, world: &World) {
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

/// Describes the scheduling constraints of a system.
#[derive(Debug, Clone)]
pub struct Schedule<S: Stage> {
    /// The stage the system should execute within.
    pub stage: S,
    /// The systems which should run before the system.
    pub to_run_before: Vec<SystemId>,
    /// The systems which should run after the system.
    pub to_run_after: Vec<SystemId>,
}

/// Scheduled the execution of systems within stages.
///
/// Stages define phases of an update loop; e.g. "begin", "update", "draw", "end".
///
/// Each stage is executed sequentially, with system command buffers flushed at the end
/// of each stage.
///
/// If the `par-schedule` feature is enabled, systems within a stage may be scheduled concurrently.
/// Dependency ordering is guarenteed only in terms of the order in which reads and writes to
/// resources and entities might be observed.
///
/// Every system within a scheduler must have a unique system ID.
pub struct SystemScheduler<S: Stage> {
    _stage: PhantomData<S>,
    dependencies: HashMap<SystemId, Schedule<S>>,
    scheduled: Vec<(S, StageExecutor)>,
    unscheduled: Vec<Box<dyn Schedulable>>,
}

impl<S: Stage> SystemScheduler<S> {
    /// Creates a new system scheduler.
    pub fn new() -> Self { Self::default() }

    /// Adds a system to the scheduler.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same `SystemId` is already scheduled.
    pub fn add_system<T: Into<Box<dyn Schedulable>>>(&mut self, stage: S, system: T) {
        self.add_systems_with_deps(stage, vec![system.into()], vec![], vec![]);
    }

    /// Adds a system to the scheduler with ordering dependencies on other systems.
    ///
    /// `to_run_before` names systems which must run before this system.  
    /// `to_run_after` names systems which must run after this system.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same `SystemId` is already scheduled.
    pub fn add_system_with_deps<T: Into<Box<dyn Schedulable>>>(
        &mut self,
        stage: S,
        system: T,
        to_run_before: Vec<SystemId>,
        to_run_after: Vec<SystemId>,
    ) {
        self.add_systems_with_deps(stage, vec![system.into()], to_run_before, to_run_after);
    }

    /// Adds multiple systems to the scheduler. Each system is executed in the order given.
    ///
    /// `to_run_before` names systems which must run before these systems.  
    /// `to_run_after` names systems which must run after these systems.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same `SystemId` is already scheduled.
    pub fn add_systems(&mut self, stage: S, systems: Vec<Box<dyn Schedulable>>) {
        self.add_systems_with_deps(stage, systems, vec![], vec![]);
    }

    /// Adds multiple systems to the scheduler with ordering dependencies on other systems.
    /// Each system is executed in the order given.
    ///
    /// `to_run_before` names systems which must run before these systems.  
    /// `to_run_after` names systems which must run after these systems.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same `SystemId` is already scheduled.
    pub fn add_systems_with_deps(
        &mut self,
        stage: S,
        systems: Vec<Box<dyn Schedulable>>,
        to_run_before: Vec<SystemId>,
        to_run_after: Vec<SystemId>,
    ) {
        self.add(
            systems,
            Schedule {
                stage,
                to_run_before,
                to_run_after,
            },
        );
    }

    /// Adds systems to the scheduler.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same `SystemId` is already scheduled.
    pub fn add(&mut self, systems: Vec<Box<dyn Schedulable>>, schedule: Schedule<S>) {
        for i in 0..systems.len() {
            let mut to_run_before = schedule.to_run_before.clone();
            let mut to_run_after = schedule.to_run_after.clone();

            if i > 0 {
                to_run_before.push(systems[i - 1].name().clone());
            }

            if i < systems.len() - 1 {
                to_run_after.push(systems[i + 1].name().clone());
            }

            let id = systems[i].name();
            if self.dependencies.contains_key(&id) {
                panic!("A system with identifier \"{}\" already exists", id);
            }

            self.dependencies.insert(
                id.clone(),
                Schedule {
                    stage: schedule.stage,
                    to_run_before,
                    to_run_after,
                },
            );
        }

        for system in systems {
            self.unscheduled.push(system);
        }
    }

    /// Removes a system from the scheduler.
    pub fn remove(&mut self, id: SystemId) -> Option<(Box<dyn Schedulable>, Schedule<S>)> {
        if let Some(schedule) = self.dependencies.remove(&id) {
            if let Ok(executor_index) = self
                .scheduled
                .binary_search_by_key(&&schedule.stage, |(s, _)| s)
            {
                let (stage, executor) = self.scheduled.remove(executor_index);
                let mut systems = executor.into_vec();
                let system_index = systems.iter().position(|s| s.name() == &id).unwrap();
                let result = systems.remove(system_index);

                let executor = StageExecutor::new(systems);
                self.scheduled.insert(executor_index, (stage, executor));
                return Some((result, schedule));
            }
        }

        None
    }

    /// Converts this scheduler into a vector of systems and their schedules.
    pub fn into_vec(mut self) -> Vec<(Box<dyn Schedulable>, Schedule<S>)> {
        self.construct_stages();
        let mut result = Vec::new();
        let mut scheduled = self.scheduled;
        let mut dependencies = self.dependencies;
        for (_, executor) in scheduled.drain(..) {
            let systems = executor.into_vec().into_iter().map(|sys| {
                let info = dependencies.remove(sys.name()).unwrap();
                (sys, info)
            });
            result.extend(systems);
        }

        result
    }

    /// Executes all scheduled systems.
    ///
    /// # Panics
    ///
    /// Panics if scheduled systems have impossible schedule constraints.
    pub fn execute(&mut self, world: &mut World) {
        self.construct_stages();
        for (stage, executor) in &mut self.scheduled {
            let span = span!(Level::INFO, "Running stage", %stage);
            let _guard = span.enter();
            executor.execute(world);
        }
    }

    fn construct_stages(&mut self) {
        // check if stages need to be rebuilt
        if self.unscheduled.is_empty() {
            return;
        }

        // collect new stages
        let mut systems: HashMap<_, _> = self
            .unscheduled
            .drain(..)
            .map(|s| (s.name().clone(), s))
            .collect();

        // drain existing executors
        for (_, executor) in self.scheduled.drain(..) {
            for system in executor.into_vec() {
                systems.insert(system.name().clone(), system);
            }
        }

        // collect and sort active stages
        let mut stages = self
            .dependencies
            .iter()
            .map(|(_, info)| info.stage)
            .collect::<Vec<_>>();

        stages.sort();
        stages.dedup();

        // create new stage executors
        for stage in stages {
            use petgraph::Graph;

            let mut graph = Graph::<SystemId, ()>::new();
            let mut node_to_system = HashMap::new();
            let mut system_to_node = HashMap::new();

            // add nodes to dependency graph
            for (id, _) in self
                .dependencies
                .iter()
                .filter(|(_, info)| info.stage == stage)
            {
                let index = graph.add_node(id.clone());
                node_to_system.insert(index, id.clone());
                system_to_node.insert(id.clone(), index);
            }

            // add dependency edges
            for (id, info) in self
                .dependencies
                .iter()
                .filter(|(_, info)| info.stage == stage)
            {
                for before in &info.to_run_before {
                    if let Some(s) = self.dependencies.get(before) {
                        if s.stage > stage {
                            panic!(
                                "invalid dependency: {a} requires {b} runs before it, but {b} runs in {b_stage} which is after {a_stage}",
                                a=id,
                                b=before,
                                b_stage=s.stage,
                                a_stage=stage
                            );
                        }

                        if s.stage == stage {
                            graph.add_edge(system_to_node[before], system_to_node[id], ());
                        }
                    }
                }

                for after in &info.to_run_after {
                    if let Some(s) = self.dependencies.get(after) {
                        if s.stage < stage {
                            panic!(
                                "invalid dependency: {a} requires {b} runs after it, but {b} runs in {b_stage} which is before {a_stage}",
                                a=id,
                                b=after,
                                b_stage=s.stage,
                                a_stage=stage
                            );
                        }

                        if s.stage == stage {
                            graph.add_edge(system_to_node[id], system_to_node[after], ());
                        }
                    }
                }
            }

            // sort dependencies and create executor
            match petgraph::algo::toposort(&graph, None) {
                Ok(mut order) => {
                    let systems: Vec<Box<dyn Schedulable>> = order
                        .drain(..)
                        .map(|id| systems.remove(&node_to_system[&id]).unwrap())
                        .collect();
                    let executor = StageExecutor::new(systems);
                    self.scheduled.push((stage, executor));
                }
                Err(cycle) => panic!(
                    "dependency cycle involving {}",
                    node_to_system[&cycle.node_id()]
                ),
            }
        }
    }
}

impl<S: Stage> Default for SystemScheduler<S> {
    fn default() -> Self {
        Self {
            dependencies: HashMap::new(),
            scheduled: Vec::new(),
            unscheduled: Vec::new(),
            _stage: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use itertools::sorted;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Stages {
        Begin,
        Update,
        Draw,
        End,
    }

    impl Stage for Stages {}

    impl std::fmt::Display for Stages {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Stages::Begin => write!(f, "begin"),
                Stages::Update => write!(f, "update"),
                Stages::Draw => write!(f, "draw"),
                Stages::End => write!(f, "end"),
            }
        }
    }

    #[test]
    fn stages_execution_order() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let order = Arc::new(Mutex::new(Vec::new()));

        let order_clone = order.clone();
        let system_one = SystemBuilder::new("one")
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(1usize));
        let order_clone = order.clone();
        let system_two = SystemBuilder::new("two")
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(2usize));
        let order_clone = order.clone();
        let system_three = SystemBuilder::new("three")
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(3usize));

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system(Stages::Draw, system_two);
        scheduler.add_system(Stages::Begin, system_one);
        scheduler.add_system(Stages::End, system_three);

        scheduler.execute(&mut world);

        let order = order.lock().unwrap();
        let sorted: Vec<usize> = sorted(order.clone()).collect();
        assert_eq!(*order, sorted);
    }

    #[test]
    fn deps_execution_order_before() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let order = Arc::new(Mutex::new(Vec::new()));

        #[derive(Default)]
        struct Resource;

        world.resources.insert(Resource);

        let order_clone = order.clone();
        let system_one = SystemBuilder::new("one")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(1usize));
        let order_clone = order.clone();
        let system_two = SystemBuilder::new("two")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(2usize));
        let order_clone = order.clone();
        let system_three = SystemBuilder::new("three")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(3usize));

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(Stages::Begin, system_two, vec!["one".into()], vec![]);
        scheduler.add_system(Stages::Begin, system_one);
        scheduler.add_system_with_deps(Stages::Begin, system_three, vec!["two".into()], vec![]);

        scheduler.execute(&mut world);

        let order = order.lock().unwrap();
        let sorted: Vec<usize> = sorted(order.clone()).collect();
        assert_eq!(*order, sorted);
    }

    #[test]
    fn deps_execution_order_after() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let order = Arc::new(Mutex::new(Vec::new()));

        #[derive(Default)]
        struct Resource;

        world.resources.insert(Resource);

        let order_clone = order.clone();
        let system_one = SystemBuilder::new("one")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(1usize));
        let order_clone = order.clone();
        let system_two = SystemBuilder::new("two")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(2usize));
        let order_clone = order.clone();
        let system_three = SystemBuilder::new("three")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(3usize));

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(Stages::Begin, system_two, vec![], vec!["three".into()]);
        scheduler.add_system_with_deps(Stages::Begin, system_one, vec![], vec!["two".into()]);
        scheduler.add_system(Stages::Begin, system_three);

        scheduler.execute(&mut world);

        let order = order.lock().unwrap();
        let sorted: Vec<usize> = sorted(order.clone()).collect();
        assert_eq!(*order, sorted);
    }

    #[test]
    fn deps_execution_order_across_stages() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let order = Arc::new(Mutex::new(Vec::new()));

        #[derive(Default)]
        struct Resource;

        world.resources.insert(Resource);

        let order_clone = order.clone();
        let system_one = SystemBuilder::new("one")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(1usize));
        let order_clone = order.clone();
        let system_two = SystemBuilder::new("two")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(2usize));
        let order_clone = order.clone();
        let system_three = SystemBuilder::new("three")
            .write_resource::<Resource>()
            .build(move |_, _, _, _| order_clone.lock().unwrap().push(3usize));

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(
            Stages::Update,
            system_two,
            vec!["one".into()],
            vec!["three".into()],
        );
        scheduler.add_system_with_deps(
            Stages::Begin,
            system_one,
            vec![],
            vec!["two".into(), "three".into()],
        );
        scheduler.add_system_with_deps(Stages::End, system_three, vec!["two".into()], vec![]);

        scheduler.execute(&mut world);

        let order = order.lock().unwrap();
        let sorted: Vec<usize> = sorted(order.clone()).collect();
        assert_eq!(*order, sorted);
    }

    #[test]
    #[should_panic(expected = "dependency cycle involving")]
    fn deps_cycle_panics() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let system_one = SystemBuilder::new("one").build(move |_, _, _, _| {});
        let system_two = SystemBuilder::new("two").build(move |_, _, _, _| {});

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(Stages::Begin, system_one, vec!["two".into()], vec![]);
        scheduler.add_system_with_deps(Stages::Begin, system_two, vec!["one".into()], vec![]);

        scheduler.execute(&mut world);
    }

    #[test]
    #[should_panic(expected = "invalid dependency:")]
    fn deps_incorrect_stage_order_earlier() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let system_one = SystemBuilder::new("one").build(move |_, _, _, _| {});
        let system_two = SystemBuilder::new("two").build(move |_, _, _, _| {});

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(Stages::Begin, system_one, vec!["two".into()], vec![]);
        scheduler.add_system_with_deps(Stages::End, system_two, vec![], vec![]);

        scheduler.execute(&mut world);
    }

    #[test]
    #[should_panic(expected = "invalid dependency:")]
    fn deps_incorrect_stage_order_later() {
        let universe = Universe::new();
        let mut world = universe.create_world();

        let system_one = SystemBuilder::new("one").build(move |_, _, _, _| {});
        let system_two = SystemBuilder::new("two").build(move |_, _, _, _| {});

        let mut scheduler = SystemScheduler::new();
        scheduler.add_system_with_deps(Stages::Begin, system_one, vec![], vec![]);
        scheduler.add_system_with_deps(Stages::End, system_two, vec![], vec!["one".into()]);

        scheduler.execute(&mut world);
    }
}
