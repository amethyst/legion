//! Contains types related to defining system schedules.

use std::cell::UnsafeCell;

#[cfg(feature = "parallel")]
use tracing::{span, trace, Level};

#[cfg(feature = "parallel")]
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[cfg(feature = "parallel")]
use itertools::izip;

use super::{
    command::CommandBuffer,
    resources::{ResourceTypeId, Resources, UnsafeResources},
    system::SystemId,
};
use crate::internals::{
    storage::component::ComponentTypeId,
    subworld::ArchetypeAccess,
    world::{World, WorldId},
};
#[cfg(feature = "parallel")]
use std::iter::repeat;

/// A `Runnable` which is also `Send` and `Sync`.
pub trait ParallelRunnable: Runnable + Send + Sync {}

impl<T: Runnable + Send + Sync> ParallelRunnable for T {}

/// Trait describing a schedulable type. This is implemented by `System`
pub trait Runnable {
    /// Gets the name of the system.
    fn name(&self) -> Option<&SystemId>;

    /// Gets the resources and component types read by the system.
    fn reads(&self) -> (&[ResourceTypeId], &[ComponentTypeId]);

    /// Gets the resources and component types written by the system.
    fn writes(&self) -> (&[ResourceTypeId], &[ComponentTypeId]);

    /// Prepares the system for execution against a world.
    fn prepare(&mut self, world: &World);

    /// Gets the set of archetypes the system will access when run,
    /// as determined when the system was last prepared.
    fn accesses_archetypes(&self) -> &ArchetypeAccess;

    /// Runs the system.
    ///
    /// # Safety
    ///
    /// The shared references to world and resources may result in unsound mutable aliasing if other code
    /// is accessing the same components or resources as this system. Prefer to use `run` when possible.
    ///
    /// Additionally, systems which are !Sync should never be invoked on a different thread to that which
    /// owns the resources collection passed into this function.
    unsafe fn run_unsafe(&mut self, world: &World, resources: &UnsafeResources);

    /// Gets the system's command buffer.
    fn command_buffer_mut(&mut self, world: WorldId) -> Option<&mut CommandBuffer>;

    /// Runs the system.
    fn run(&mut self, world: &mut World, resources: &mut Resources) {
        unsafe { self.run_unsafe(world, resources.internal()) };
    }
}

/// Executes a sequence of systems, potentially in parallel, and then commits their command buffers.
///
/// Systems are provided in execution order. When the `parallel` feature is enabled, the `Executor`
/// may run some systems in parallel. The order in which side-effects (e.g. writes to resources
/// or entities) are observed is maintained.
pub struct Executor {
    systems: Vec<SystemBox>,
    #[cfg(feature = "parallel")]
    static_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "parallel")]
    dynamic_dependants: Vec<Vec<usize>>,
    #[cfg(feature = "parallel")]
    static_dependency_counts: Vec<AtomicUsize>,
    #[cfg(feature = "parallel")]
    awaiting: Vec<AtomicUsize>,
}

struct SystemBox(UnsafeCell<Box<dyn ParallelRunnable>>);

// NOT SAFE:
// This type is only safe to use as Send and Sync within
// the constraints of how it is used inside Executor
unsafe impl Send for SystemBox {}
unsafe impl Sync for SystemBox {}

impl SystemBox {
    #[cfg(feature = "parallel")]
    unsafe fn get(&self) -> &dyn ParallelRunnable {
        std::ops::Deref::deref(&*self.0.get())
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn get_mut(&self) -> &mut dyn ParallelRunnable {
        std::ops::DerefMut::deref_mut(&mut *self.0.get())
    }
}

impl Executor {
    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    #[cfg(not(feature = "parallel"))]
    pub fn new(systems: Vec<Box<dyn ParallelRunnable>>) -> Self {
        Self {
            systems: systems
                .into_iter()
                .map(|s| SystemBox(UnsafeCell::new(s)))
                .collect(),
        }
    }

    /// Constructs a new executor for all systems to be run in a single stage.
    ///
    /// Systems are provided in the order in which side-effects (e.g. writes to resources or entities)
    /// are to be observed.
    #[cfg(feature = "parallel")]
    #[allow(clippy::cognitive_complexity)]
    // TODO: we should break this up
    pub fn new(systems: Vec<Box<dyn ParallelRunnable>>) -> Self {
        if systems.len() > 1 {
            let mut static_dependency_counts = Vec::with_capacity(systems.len());

            let mut static_dependants: Vec<Vec<_>> =
                repeat(Vec::with_capacity(64)).take(systems.len()).collect();
            let mut dynamic_dependants: Vec<Vec<_>> =
                repeat(Vec::with_capacity(64)).take(systems.len()).collect();

            #[derive(Default)]
            struct PreviousAccess {
                readers: Vec<usize>,
                last_writer: Option<usize>,
            }

            impl PreviousAccess {
                fn add_read(&mut self, idx: usize) -> Option<usize> {
                    self.readers.push(idx);
                    self.last_writer
                }

                fn add_write(&mut self, idx: usize) -> Vec<usize> {
                    let mut dependencies = Vec::new();
                    std::mem::swap(&mut self.readers, &mut dependencies);
                    if let Some(writer) = self.last_writer.replace(idx) {
                        dependencies.push(writer)
                    }
                    dependencies
                }
            }

            let mut resource_accesses =
                HashMap::<ResourceTypeId, PreviousAccess>::with_capacity_and_hasher(
                    64,
                    Default::default(),
                );
            let mut component_accesses =
                HashMap::<ComponentTypeId, PreviousAccess>::with_capacity_and_hasher(
                    64,
                    Default::default(),
                );

            for (i, system) in systems.iter().enumerate() {
                let span = if let Some(name) = system.name() {
                    span!(
                        Level::TRACE,
                        "Building system dependencies",
                        system = %name,
                        index = i,
                    )
                } else {
                    span!(Level::TRACE, "building system dependencies", index = i)
                };
                let _guard = span.enter();

                let (read_res, read_comp) = system.reads();
                let (write_res, write_comp) = system.writes();

                // find resource access dependencies
                let mut dependencies = HashSet::with_capacity(64);
                for res in read_res {
                    let access = resource_accesses.entry(*res).or_default();
                    if let Some(dep) = access.add_read(i) {
                        dependencies.insert(dep);
                    }
                }
                for res in write_res {
                    let access = resource_accesses.entry(*res).or_default();
                    for dep in access.add_write(i) {
                        dependencies.insert(dep);
                    }
                }

                static_dependency_counts.push(AtomicUsize::from(dependencies.len()));
                trace!(dependants = ?dependencies, dependency_counts = ?static_dependency_counts, "Computed static dependants");
                for dep in &dependencies {
                    static_dependants[*dep].push(i);
                }

                // find component access dependencies
                let mut comp_dependencies = HashSet::<usize>::default();
                for comp in read_comp {
                    let access = component_accesses.entry(*comp).or_default();
                    if let Some(dep) = access.add_read(i) {
                        comp_dependencies.insert(dep);
                    }
                }
                for comp in write_comp {
                    let access = component_accesses.entry(*comp).or_default();
                    for dep in access.add_write(i) {
                        comp_dependencies.insert(dep);
                    }
                }

                // remove dependencies which are already static from dynamic dependencies
                for static_dep in &dependencies {
                    comp_dependencies.remove(static_dep);
                }

                trace!(depentants = ?comp_dependencies, "Computed dynamic dependants");
                for dep in comp_dependencies {
                    if dep != i {
                        // dont be dependent on ourselves
                        dynamic_dependants[dep].push(i);
                    }
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

            Executor {
                awaiting,
                static_dependants,
                dynamic_dependants,
                static_dependency_counts,
                systems: systems
                    .into_iter()
                    .map(|s| SystemBox(UnsafeCell::new(s)))
                    .collect(),
            }
        } else {
            Executor {
                awaiting: Vec::with_capacity(0),
                static_dependants: Vec::with_capacity(0),
                dynamic_dependants: Vec::with_capacity(0),
                static_dependency_counts: Vec::with_capacity(0),
                systems: systems
                    .into_iter()
                    .map(|s| SystemBox(UnsafeCell::new(s)))
                    .collect(),
            }
        }
    }

    /// Converts this executor into a vector of its component systems.
    pub fn into_vec(self) -> Vec<Box<dyn ParallelRunnable>> {
        self.systems.into_iter().map(|s| s.0.into_inner()).collect()
    }

    /// Executes all systems and then flushes their command buffers.
    #[cfg(not(feature = "parallel"))]
    pub fn execute(&mut self, world: &mut World, resources: &mut Resources) {
        let resources = resources.internal();
        self.run_systems(world, resources);
        self.flush_command_buffers(world);
    }

    /// Executes all systems and then flushes their command buffers.
    #[cfg(feature = "parallel")]
    pub fn execute(&mut self, world: &mut World, resources: &mut Resources) {
        let resources = resources.internal();
        rayon::join(|| self.run_systems(world, resources), || {});
        self.flush_command_buffers(world);
    }

    /// Executes all systems sequentially.
    ///
    /// Only enabled with parallel is disabled
    #[cfg(not(feature = "parallel"))]
    pub fn run_systems(&mut self, world: &mut World, resources: &UnsafeResources) {
        self.systems.iter_mut().for_each(|system| {
            let system = unsafe { system.get_mut() };
            system.prepare(world);
            unsafe { system.run_unsafe(world, resources) };
        });
    }

    /// Executes all systems, potentially in parallel.
    ///
    /// Ordering is retained in so far as the order of observed resource and component
    /// accesses is maintained.
    ///
    /// Call from within `rayon::ThreadPool::install()` to execute within a specific thread pool.
    #[cfg(feature = "parallel")]
    pub fn run_systems(&mut self, world: &mut World, resources: &UnsafeResources) {
        match self.systems.len() {
            1 => {
                // safety: we have exlusive access to all systems, world and resources here
                unsafe {
                    let system = self.systems[0].get_mut();
                    system.prepare(world);
                    system.run_unsafe(world, resources);
                };
            }
            _ => {
                let systems = &mut self.systems;
                let static_dependency_counts = &self.static_dependency_counts;
                let awaiting = &mut self.awaiting;

                // prepare all systems - archetype filters are pre-executed here
                systems
                    .par_iter_mut()
                    .for_each(|sys| unsafe { sys.get_mut() }.prepare(world));

                // determine dynamic dependencies
                izip!(
                    systems.iter(),
                    self.static_dependants.iter_mut(),
                    self.dynamic_dependants.iter_mut()
                )
                .par_bridge()
                .for_each(|(sys, static_dep, dyn_dep)| {
                    // safety: systems is held exclusively, and we are only reading each system
                    let archetypes = unsafe { sys.get() }.accesses_archetypes();
                    for i in (0..dyn_dep.len()).rev() {
                        let dep = dyn_dep[i];
                        let other = unsafe { systems[dep].get() };

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

                trace!(?awaiting, "Initialized await counts");

                // execute all systems with no outstanding dependencies
                (0..systems.len())
                    .into_par_iter()
                    .filter(|i| static_dependency_counts[*i].load(Ordering::SeqCst) == 0)
                    .for_each(|i| {
                        // safety: we are at the root of the execution tree, so we know each
                        // index is exclusive here
                        unsafe { self.run_recursive(i, world, resources) };
                    });

                debug_assert!(
                    awaiting.iter().all(|x| x.load(Ordering::SeqCst) == 0),
                    "not all systems run: {:?}",
                    awaiting
                );
            }
        }
    }

    /// Flushes the recorded command buffers for all systems.
    pub fn flush_command_buffers(&mut self, world: &mut World) {
        self.systems.iter().for_each(|system| {
            // safety: systems are exlcusive due to &mut self
            let system = unsafe { system.get_mut() };
            if let Some(cmd) = system.command_buffer_mut(world.id()) {
                cmd.flush(world);
            }
        });
    }

    /// Recursively execute through the generated depedency cascade and exhaust it.
    ///
    /// # Safety
    ///
    /// Ensure the system indexed by `i` is only accessed once.
    #[cfg(feature = "parallel")]
    unsafe fn run_recursive(&self, i: usize, world: &World, resources: &UnsafeResources) {
        // safety: the caller ensures nothing else is accessing systems[i]
        self.systems[i].get_mut().run_unsafe(world, resources);

        self.static_dependants[i].par_iter().for_each(|dep| {
            if self.awaiting[*dep].fetch_sub(1, Ordering::Relaxed) == 1 {
                // safety: each dependency is unique, so run_recursive is safe to call
                self.run_recursive(*dep, world, resources);
            }
        });
    }
}

/// A factory for `Schedule`.
pub struct Builder {
    steps: Vec<Step>,
    accumulator: Vec<Box<dyn ParallelRunnable>>,
}

impl Builder {
    /// Adds a system to the schedule.
    pub fn add_system<T: ParallelRunnable + 'static>(&mut self, system: T) -> &mut Self {
        self.accumulator.push(Box::new(system));
        self
    }

    /// Waits for executing systems to complete, and the flushes all outstanding system
    /// command buffers.
    pub fn flush(&mut self) -> &mut Self {
        self.finalize_executor();
        self.steps.push(Step::FlushCmdBuffers);
        self
    }

    fn finalize_executor(&mut self) {
        if !self.accumulator.is_empty() {
            let mut systems = Vec::new();
            std::mem::swap(&mut self.accumulator, &mut systems);
            let executor = Executor::new(systems);
            self.steps.push(Step::Systems(executor));
        }
    }

    /// Adds a thread local function to the schedule. This function will be executed on the main thread.
    pub fn add_thread_local_fn<F: FnMut(&mut World, &mut Resources) + 'static>(
        &mut self,
        f: F,
    ) -> &mut Self {
        self.finalize_executor();
        self.steps.push(Step::ThreadLocalFn(
            Box::new(f) as Box<dyn FnMut(&mut World, &mut Resources)>
        ));
        self
    }

    /// Adds a thread local system to the schedule. This system will be executed on the main thread.
    pub fn add_thread_local<S: Runnable + 'static>(&mut self, system: S) -> &mut Self {
        self.finalize_executor();
        let system = Box::new(system) as Box<dyn Runnable>;
        self.steps.push(Step::ThreadLocalSystem(system));
        self
    }

    /// Finalizes the builder into a `Schedule`.
    pub fn build(&mut self) -> Schedule {
        self.flush();
        let mut steps = Vec::new();
        std::mem::swap(&mut self.steps, &mut steps);
        Schedule { steps }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            accumulator: Vec::new(),
        }
    }
}

/// A step in a schedule.
pub enum Step {
    /// A batch of systems.
    Systems(Executor),
    /// Flush system command buffers.
    FlushCmdBuffers,
    /// A thread local function.
    ThreadLocalFn(Box<dyn FnMut(&mut World, &mut Resources)>),
    /// A thread local system
    ThreadLocalSystem(Box<dyn Runnable>),
}

/// A schedule of systems for execution.
///
/// # Examples
///
/// ```rust
/// # use legion::*;
/// # let find_collisions = SystemBuilder::new("find_collisions").build(|_,_,_,_| {});
/// # let calculate_acceleration = SystemBuilder::new("calculate_acceleration").build(|_,_,_,_| {});
/// # let update_positions = SystemBuilder::new("update_positions").build(|_,_,_,_| {});
/// let mut world = World::default();
/// let mut resources = Resources::default();
/// let mut schedule = Schedule::builder()
///     .add_system(find_collisions)
///     .flush()
///     .add_system(calculate_acceleration)
///     .add_system(update_positions)
///     .build();
///
/// schedule.execute(&mut world, &mut resources);
/// ```
pub struct Schedule {
    steps: Vec<Step>,
}

impl Schedule {
    /// Creates a new schedule builder.
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Executes all of the steps in the schedule.
    #[cfg(not(feature = "parallel"))]
    pub fn execute(&mut self, world: &mut World, resources: &mut Resources) {
        self.execute_internal(world, resources, |world, resources, executor| {
            executor.run_systems(world, resources.internal())
        });
    }

    /// Executes all of the steps in the schedule.
    #[cfg(feature = "parallel")]
    pub fn execute(&mut self, world: &mut World, resources: &mut Resources) {
        self.execute_internal(world, resources, |world, resources, executor| {
            let resources = resources.internal();
            rayon::join(|| executor.run_systems(world, resources), || {});
        });
    }

    /// Executes all of the steps in the schedule, with parallelized systems running in
    /// the given thread pool.
    #[cfg(feature = "parallel")]
    pub fn execute_in_thread_pool(
        &mut self,
        world: &mut World,
        resources: &mut Resources,
        pool: &rayon::ThreadPool,
    ) {
        self.execute_internal(world, resources, |world, resources, executor| {
            let resources = resources.internal();
            pool.install(|| executor.run_systems(world, resources));
        });
    }

    fn execute_internal<F: FnMut(&mut World, &mut Resources, &mut Executor)>(
        &mut self,
        world: &mut World,
        resources: &mut Resources,
        mut run_executor: F,
    ) {
        enum ToFlush<'a> {
            Executor(&'a mut Executor),
            System(&'a mut CommandBuffer),
        }

        let mut waiting_flush: Vec<ToFlush> = Vec::new();
        for step in &mut self.steps {
            match step {
                Step::Systems(executor) => {
                    run_executor(world, resources, executor);
                    waiting_flush.push(ToFlush::Executor(executor));
                }
                Step::FlushCmdBuffers => {
                    waiting_flush.drain(..).for_each(|e| match e {
                        ToFlush::Executor(exec) => exec.flush_command_buffers(world),
                        ToFlush::System(cmd) => cmd.flush(world),
                    });
                }
                Step::ThreadLocalFn(function) => function(world, resources),
                Step::ThreadLocalSystem(system) => {
                    system.prepare(world);
                    system.run(world, resources);
                    if let Some(cmd) = system.command_buffer_mut(world.id()) {
                        waiting_flush.push(ToFlush::System(cmd));
                    }
                }
            }
        }
    }

    /// Converts the schedule into a vector of steps.
    pub fn into_vec(self) -> Vec<Step> {
        self.steps
    }
}

impl From<Builder> for Schedule {
    fn from(mut builder: Builder) -> Self {
        builder.build()
    }
}

impl From<Vec<Step>> for Schedule {
    fn from(steps: Vec<Step>) -> Self {
        Self { steps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internals::{
        query::{view::write::Write, IntoQuery},
        systems::system::SystemBuilder,
    };
    use itertools::sorted;
    use std::sync::{Arc, Mutex};

    #[test]
    fn execute_in_order() {
        let mut world = World::default();

        #[derive(Default)]
        struct Resource;

        let mut resources = Resources::default();
        resources.insert(Resource);

        let order = Arc::new(Mutex::new(Vec::new()));

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

        let mut schedule = Schedule::builder()
            .add_system(system_one)
            .add_system(system_two)
            .add_system(system_three)
            .build();

        schedule.execute(&mut world, &mut resources);

        let order = order.lock().unwrap();
        let sorted: Vec<usize> = sorted(order.clone()).collect();
        assert_eq!(*order, sorted);
    }

    #[test]
    fn flush() {
        let mut world = World::default();
        let mut resources = Resources::default();

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct TestComp(f32, f32, f32);

        let system_one = SystemBuilder::new("one").build(move |cmd, _, _, _| {
            cmd.push((TestComp(0., 0., 0.),));
        });
        let system_two = SystemBuilder::new("two")
            .with_query(Write::<TestComp>::query())
            .build(move |_, world, _, query| {
                assert_eq!(0, query.iter_mut(world).count());
            });
        let system_three = SystemBuilder::new("three")
            .with_query(Write::<TestComp>::query())
            .build(move |_, world, _, query| {
                assert_eq!(1, query.iter_mut(world).count());
            });

        let mut schedule = Schedule::builder()
            .add_system(system_one)
            .add_system(system_two)
            .flush()
            .add_system(system_three)
            .build();

        schedule.execute(&mut world, &mut resources);
    }

    #[test]
    fn flush_thread_local() {
        let mut world = World::default();
        let mut resources = Resources::default();

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct TestComp(f32, f32, f32);

        let entity = Arc::new(Mutex::new(None));

        {
            let entity = entity.clone();

            let system_one = SystemBuilder::new("one").build(move |cmd, _, _, _| {
                let mut entity = entity.lock().unwrap();
                *entity = Some(cmd.push((TestComp(0.0, 0.0, 0.0),)));
            });

            let mut schedule = Schedule::builder().add_thread_local(system_one).build();

            schedule.execute(&mut world, &mut resources);
        }

        let entity = entity.lock().unwrap();

        assert!(entity.is_some());
        assert!(world.entry(entity.unwrap()).is_some());
    }

    #[test]
    fn thread_local_resource() {
        let mut world = World::default();
        let mut resources = Resources::default();

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct NotSync(*const u8);

        resources.insert(NotSync(std::ptr::null()));

        let system = SystemBuilder::new("one")
            .read_resource::<NotSync>()
            .build(move |_, _, _, _| {});

        let mut schedule = Schedule::builder().add_thread_local(system).build();

        // this should not compile
        // let mut schedule = Schedule::builder().add_system(system).build();

        schedule.execute(&mut world, &mut resources);
    }
}
