use crate::borrow::{AtomicRefCell, Exclusive, Ref, RefMut, Shared};
use crate::command::CommandBuffer;
use crate::cons::{ConsAppend, ConsFlatten};
use crate::entity::Entity;
use crate::filter::EntityFilter;
use crate::query::{
    Chunk, ChunkDataIter, ChunkEntityIter, ChunkViewIter, Query, Read, View, Write,
};
use crate::resource::{Resource, ResourceSet};
use crate::schedule::{Runnable, Schedulable};
use crate::storage::{Component, ComponentTypeId, TagTypeId};
use crate::world::World;
use bit_set::BitSet;
use derivative::Derivative;
use rayon::prelude::*;
use shrinkwraprs::Shrinkwrap;
use std::any::TypeId;
use std::marker::PhantomData;

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
// FIXME: This would have an associated lifetime and would hold references instead of pointers,
// but this is a workaround for lack of GATs and bugs around HRTBs combined with associated types.
// See https://github.com/rust-lang/rust/issues/62529
pub struct PreparedQuery<V, F>
where
    V: for<'v> View<'v>,
    F: EntityFilter,
{
    world: *const World,
    query: *mut Query<V, F>,
}

unsafe impl<V, F> Send for PreparedQuery<V, F>
where
    V: for<'v> View<'v>,
    F: EntityFilter,
{
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
    #[inline]
    pub fn iter_chunks<'a, 'b>(
        &'b mut self,
    ) -> ChunkViewIter<'a, 'b, V, F::ArchetypeFilter, F::ChunksetFilter, F::ChunkFilter> {
        unsafe { (&mut *self.query).iter_chunks(&*self.world) }
    }

    /// Gets an iterator which iterates through all entity data that matches the query, and also yields the the `Entity` IDs.
    #[inline]
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
    #[inline]
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
    #[inline]
    pub fn for_each<'a, 'data, T>(&'a mut self, mut f: T)
    where
        T: Fn(<<V as View<'data>>::Iter as Iterator>::Item),
    {
        unsafe { (&mut *self.query).for_each(&*self.world, f) }
    }

    /// Iterates through all entities that matches the query in parallel by chunk
    #[inline]
    pub fn par_entities_for_each<'a, T>(&'a mut self, f: T)
    where
        T: Fn((Entity, <<V as View<'a>>::Iter as Iterator>::Item)) + Send + Sync,
    {
        unsafe { (&mut *self.query).par_entities_for_each(&*self.world, f) }
    }

    /// Iterates through all entity data that matches the query in parallel.
    #[cfg(feature = "par-iter")]
    #[inline]
    pub fn par_for_each<'a, T>(&'a mut self, f: T)
    where
        T: Fn(<<V as View<'a>>::Iter as Iterator>::Item) + Send + Sync,
    {
        unsafe { (&mut *self.query).par_for_each(&*self.world, f) }
    }

    /// Gets a parallel iterator of chunks that match the query.
    #[cfg(feature = "par-iter")]
    #[inline]
    pub fn par_for_each_chunk<'a, T>(&'a mut self, world: &'a World, f: T)
    where
        T: Fn(Chunk<'a, V>) + Send + Sync,
    {
        unsafe { (&mut *self.query).par_for_each_chunk(&*self.world, f) }
    }
}

/// This trait is for providing abstraction across tuples of queries for populating the type
/// information in the system closure. This trait also provides access to the underlying query
/// information.
pub trait QuerySet: Send + Sync {
    type PreparedQueries;

    /// Returns the archetypes accessed by this collection of queries. This allows for caching
    /// effiency and granularity for system dispatching.
    fn filter_archetypes(&mut self, world: &World, archetypes: &mut BitSet);

    /// # Safety
    /// prepare call doesn't respect lifetimes of `self` and `world`.
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

impl QuerySet for () {
    type PreparedQueries = ();
    fn filter_archetypes(&mut self, _: &World, _: &mut BitSet) {}
    unsafe fn prepare(&mut self, _: &World) {}
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
impl_queryset_tuple!(A, B, C, D, E, F, G, H);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y);
impl_queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

/// Wrapper type around `World` for providing safe global access to a component for a system.
/// This is mainly useful when needing to conduct sparse access of a component.
// This wrapper is safe because the dispatcher flags this as as global access on the component type.
pub struct PreparedWorld {
    world: *const World,
    access: *const Access<ComponentTypeId>,
}
impl PreparedWorld {
    unsafe fn new(world: &World, access: &Access<ComponentTypeId>) -> Self {
        Self {
            world: world as *const World,
            access: access as *const Access<ComponentTypeId>,
        }
    }
}

unsafe impl Sync for PreparedWorld {}
unsafe impl Send for PreparedWorld {}

// TODO: these assertions should have better errors
impl PreparedWorld {
    /// Borrows component data for the given entity.
    ///
    /// Returns `Some(data)` if the entity was found and contains the specified data.
    /// Otherwise `None` is returned.
    ///
    /// # Panics
    ///
    /// This function borrows all components of type `T` in the world. It may panic if
    /// any other code is currently borrowing `T` mutable or if the component was not declared
    /// as written by this system.
    #[inline]
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<Ref<Shared, T>> {
        if !unsafe { (&*self.access) }
            .reads
            .contains(&ComponentTypeId::of::<T>())
        {
            panic!("You attempted to fetch a component which was not declared with `read_component` on the system. \
            Use SystemBuilder::read_component to add this access to: {}", std::any::type_name::<T>())
        }

        unsafe { (&*self.world) }.get_component::<T>(entity)
    }

    /// Borrows component data for the given entity.
    ///
    /// Returns `Some(data)` if the entity was found and contains the specified data.
    /// Otherwise `None` is returned.
    ///
    /// # Panics
    ///
    /// This function borrows all components of type `T` in the world. It may panic if
    /// any other code is currently borrowing `T` mutable or if the component was not declared
    /// as written by this system.
    #[inline]
    pub fn get_component_mut<T: Component>(&self, entity: Entity) -> Option<RefMut<Exclusive, T>> {
        if !unsafe { (&*self.access) }
            .writes
            .contains(&ComponentTypeId::of::<T>())
        {
            panic!("You attempted to fetch a mutable component which was not declared with `write_component` on the system. \
            Use SystemBuilder::write_component to add this access to: {}", std::any::type_name::<T>())
        }

        unsafe { (&*self.world) }.get_component_mut::<T>(entity)
    }
}

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
    R: ResourceSet,
    Q: QuerySet,
    F: SystemDisposable<Resources = R, Queries = Q>,
{
    name: String,
    resources: R,
    queries: AtomicRefCell<Q>,
    run_fn: AtomicRefCell<F>,
    archetypes: BitSet,

    // These are stored statically instead of always iterated and created from the
    // query types, which would make allocations every single request
    access: SystemAccess,

    // We pre-allocate a commnad buffer for ourself. Writes are self-draining so we never have to rellocate.
    command_buffer: AtomicRefCell<CommandBuffer>,
}

impl<R, Q, F> Runnable for System<R, Q, F>
where
    R: ResourceSet,
    Q: QuerySet,
    F: SystemDisposable<Resources = R, Queries = Q>,
{
    fn name(&self) -> &str { &self.name }

    fn reads(&self) -> (&[TypeId], &[ComponentTypeId]) {
        (&self.access.resources.reads, &self.access.components.reads)
    }
    fn writes(&self) -> (&[TypeId], &[ComponentTypeId]) {
        (
            &self.access.resources.writes,
            &self.access.components.writes,
        )
    }

    fn prepare(&mut self, world: &World) {
        self.queries
            .get_mut()
            .filter_archetypes(world, &mut self.archetypes);
    }

    fn accesses_archetypes(&self) -> &BitSet { &self.archetypes }

    fn command_buffer_mut(&self) -> RefMut<Exclusive, CommandBuffer> {
        self.command_buffer.get_mut()
    }

    fn run(&self, world: &World) {
        let mut resources = R::fetch(&world.resources);
        let mut queries = self.queries.get_mut();
        let mut prepared_queries = unsafe { queries.prepare(world) };
        let mut world_shim = unsafe { PreparedWorld::new(world, &self.access.components) };

        // Give the command buffer a new entity block.
        // This should usually just pull a free block, or allocate a new one...
        // TODO: The BlockAllocator should *ensure* keeping at least 1 free block so this prevents an allocation

        use std::ops::DerefMut;
        let mut borrow = self.run_fn.get_mut();
        SystemDisposable::run(
            borrow.deref_mut(),
            &mut self.command_buffer.get_mut(),
            &mut world_shim,
            &mut resources,
            &mut prepared_queries,
        );
    }

    fn dispose(self: Box<Self>, world: &mut World) {
        SystemDisposable::dispose(self.run_fn.into_inner(), world);
    }
}

/// Supertrait used for defining systems. All wrapper objects for systems implement this trait.
///
/// This trait will generally not be used by users.
pub trait SystemDisposable {
    type Resources: ResourceSet;
    type Queries: QuerySet;

    fn run(
        &mut self,
        commands: &mut CommandBuffer,
        world: &mut PreparedWorld,
        resources: &mut <Self::Resources as ResourceSet>::PreparedResources,
        queries: &mut <Self::Queries as QuerySet>::PreparedQueries,
    );

    fn dispose(self, world: &mut World);
}

// Wrapper type for storing disposable systems
struct SystemDisposableFnMut<
    R: ResourceSet,
    Q: QuerySet,
    F: FnMut(
            &mut CommandBuffer,
            &mut PreparedWorld,
            &mut <R as ResourceSet>::PreparedResources,
            &mut <Q as QuerySet>::PreparedQueries,
        ) + 'static,
>(F, PhantomData<(R, Q)>);

impl<R, Q, F> SystemDisposable for SystemDisposableFnMut<R, Q, F>
where
    R: ResourceSet,
    Q: QuerySet,
    F: FnMut(
            &mut CommandBuffer,
            &mut PreparedWorld,
            &mut <R as ResourceSet>::PreparedResources,
            &mut <Q as QuerySet>::PreparedQueries,
        ) + 'static,
{
    type Resources = R;
    type Queries = Q;

    fn run(
        &mut self,
        commands: &mut CommandBuffer,
        world: &mut PreparedWorld,
        resources: &mut <R as ResourceSet>::PreparedResources,
        queries: &mut <Q as QuerySet>::PreparedQueries,
    ) {
        (self.0)(commands, world, resources, queries)
    }

    fn dispose(self, _: &mut World) {}
}

// Wrapper type for state storage to be saved
#[derive(Shrinkwrap)]
#[shrinkwrap(mutable)]
struct StateWrapper<T>(pub T);
// This is safe because systems are never called from 2 threads simultaneously.
unsafe impl<T: Send> Sync for StateWrapper<T> {}

// Wrapper type for storing disposable systems
struct SystemDisposableState<
    S,
    R: ResourceSet,
    Q: QuerySet,
    F: FnMut(
            &mut S,
            &mut CommandBuffer,
            &mut PreparedWorld,
            &mut <R as ResourceSet>::PreparedResources,
            &mut <Q as QuerySet>::PreparedQueries,
        ) + 'static,
    D: FnOnce(S, &mut World) + 'static,
>(F, D, StateWrapper<S>, PhantomData<(R, Q)>);

impl<S, R, Q, F, D> SystemDisposable for SystemDisposableState<S, R, Q, F, D>
where
    R: ResourceSet,
    Q: QuerySet,
    F: FnMut(
            &mut S,
            &mut CommandBuffer,
            &mut PreparedWorld,
            &mut <R as ResourceSet>::PreparedResources,
            &mut <Q as QuerySet>::PreparedQueries,
        ) + 'static,
    D: FnOnce(S, &mut World) + 'static,
{
    type Resources = R;
    type Queries = Q;

    fn run(
        &mut self,
        commands: &mut CommandBuffer,
        world: &mut PreparedWorld,
        resources: &mut <R as ResourceSet>::PreparedResources,
        queries: &mut <Q as QuerySet>::PreparedQueries,
    ) {
        (self.0)(&mut self.2, commands, world, resources, queries)
    }

    fn dispose(self, world: &mut World) { (self.1)((self.2).0, world) }
}

// This builder uses a Cons/Hlist implemented in cons.rs to generated the static query types
// for this system. Access types are instead stored and abstracted in the top level vec here
// so the underlying ResourceSet type functions from the queries don't need to allocate.
// Otherwise, this leads to excessive alloaction for every call to reads/writes
/// The core builder of `System` types, which are systems within Legion. Systems are implemented
/// as singular closures for a given system - providing queries which should be cached for that
/// system, as well as resource access and other metadata.
/// ```rust
/// # use legion::prelude::*;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Position;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Velocity;
/// # #[derive(Copy, Clone, Debug, PartialEq)]
/// # struct Model;
/// #[derive(Copy, Clone, Debug, PartialEq)]
/// struct Static;
/// #[derive(Debug)]
/// struct TestResource {}
///
///  let mut system_one = SystemBuilder::<()>::new("TestSystem")
///            .read_resource::<TestResource>()
///            .with_query(<(Read<Position>, Tagged<Model>)>::query()
///                         .filter(!tag::<Static>() | changed::<Position>()))
///            .build(move |commands, prepared_world, resource, queries| {
///                log::trace!("Hello world");
///               let mut count = 0;
///                {
///                    for (entity, pos) in queries.iter_entities() {
///
///                    }
///                }
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
    /// Create a new system builder to construct a new system.
    ///
    /// Please note, the `name` argument for this method is just for debugging and visualization
    /// purposes and is not logically used anywhere.
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

    /// Defines a query to provide this system for its execution. Multiple queries can be provided,
    /// and queries are cached internally for efficiency for filtering and archetype ID handling.
    ///
    /// It is best practice to define your queries here, to allow for the caching to take place.
    /// These queries are then provided to the executing closure as a tuple of queries.
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

    /// Flag this resource type as being read by this system.
    ///
    /// This will inform the dispatcher to not allow any writes access to this resource while
    /// this system is running. Parralel reads still occur during execution.
    pub fn read_resource<T>(mut self) -> SystemBuilder<Q, <R as ConsAppend<Read<T>>>::Output>
    where
        T: 'static + Resource,
        R: ConsAppend<Read<T>>,
        <R as ConsAppend<Read<T>>>::Output: ConsFlatten,
    {
        self.resource_access.reads.push(TypeId::of::<T>());

        SystemBuilder {
            resources: ConsAppend::append(self.resources, Read::<T>::default()),
            name: self.name,

            queries: self.queries,
            resource_access: self.resource_access,
            component_access: self.component_access,
        }
    }

    /// Flag this resource type as being written by this system.
    ///
    /// This will inform the dispatcher to not allow any parralel access to this resource while
    /// this system is running.
    pub fn write_resource<T>(mut self) -> SystemBuilder<Q, <R as ConsAppend<Write<T>>>::Output>
    where
        T: 'static + Resource,
        R: ConsAppend<Write<T>>,
        <R as ConsAppend<Write<T>>>::Output: ConsFlatten,
    {
        self.resource_access.writes.push(TypeId::of::<T>());

        SystemBuilder {
            resources: ConsAppend::append(self.resources, Write::<T>::default()),
            name: self.name,

            queries: self.queries,
            resource_access: self.resource_access,
            component_access: self.component_access,
        }
    }

    /// This performs a soft resource block on the component for writing. The dispatcher will
    /// generally handle dispatching read and writes on components based on archetype, allowing
    /// for more granular access and more parralelization of systems.
    ///
    /// Using this method will mark the entire component as read by this system, blocking writing
    /// systems from accessing any archetypes which contain this component for the duration of its
    /// execution.
    ///
    /// This type of access with `PreparedWorld` is provided for cases where sparse component access
    /// is required and searching entire query spaces for entities is inneficient.
    pub fn read_component<T>(mut self) -> Self
    where
        T: Component,
    {
        self.component_access.reads.push(ComponentTypeId::of::<T>());

        self
    }

    /// This performs a exclusive resource block on the component for writing. The dispatcher will
    /// generally handle dispatching read and writes on components based on archetype, allowing
    /// for more granular access and more parralelization of systems.
    ///
    /// Using this method will mark the entire component as written by this system, blocking other
    /// systems from accessing any archetypes which contain this component for the duration of its
    /// execution.
    ///
    /// This type of access with `PreparedWorld` is provided for cases where sparse component access
    /// is required and searching entire query spaces for entities is inneficient.
    pub fn write_component<T>(mut self) -> Self
    where
        T: Component,
    {
        self.component_access
            .writes
            .push(ComponentTypeId::of::<T>());

        self
    }

    fn build_system_disposable<F>(self, disposable: F) -> Box<dyn Schedulable>
    where
        <R as ConsFlatten>::Output: ResourceSet + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: SystemDisposable<
                Resources = <R as ConsFlatten>::Output,
                Queries = <Q as ConsFlatten>::Output,
            > + Send
            + Sync
            + 'static,
    {
        Box::new(System {
            name: self.name,
            run_fn: AtomicRefCell::new(disposable),
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

    /// Builds a system which is considered `disposable`, which also means it carries an independent
    /// state object. This type of system construction allows for systems which may need to
    /// dispose of resources at the end of the process, which you cannot do from only `FnMut` capture
    /// variables.
    pub fn build_disposable<F, D, S>(
        self,
        initial_state: S,
        run_fn: F,
        dispose_fn: D,
    ) -> Box<dyn Schedulable>
    where
        S: Send + Sync + 'static,
        <R as ConsFlatten>::Output: ResourceSet + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: FnMut(
                &mut S,
                &mut CommandBuffer,
                &mut PreparedWorld,
                &mut <<R as ConsFlatten>::Output as ResourceSet>::PreparedResources,
                &mut <<Q as ConsFlatten>::Output as QuerySet>::PreparedQueries,
            ) + Send
            + Sync
            + 'static,
        D: FnOnce(S, &mut World) + Send + Sync + 'static,
    {
        self.build_system_disposable(SystemDisposableState(
            run_fn,
            dispose_fn,
            StateWrapper(initial_state),
            Default::default(),
        ))
    }

    /// Builds a standard legion `System`. a system is considered a closure for all purposes. This
    /// closure is `FnMut`, allowing for capture of variables for tracking state for this system.
    /// Instead of the classic OOP architecture of a system, this lets you still maintain state
    /// across execution of the systems while leveraging the type simantics of closures for better
    /// ergonomics.
    pub fn build<F>(self, run_fn: F) -> Box<dyn Schedulable>
    where
        <R as ConsFlatten>::Output: ResourceSet + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: FnMut(
                &mut CommandBuffer,
                &mut PreparedWorld,
                &mut <<R as ConsFlatten>::Output as ResourceSet>::PreparedResources,
                &mut <<Q as ConsFlatten>::Output as QuerySet>::PreparedQueries,
            ) + Send
            + Sync
            + 'static,
    {
        self.build_system_disposable(SystemDisposableFnMut(run_fn, Default::default()))
    }

    /// Builds a system which is not `Schedulable`, as it is not thread safe (!Send and !Sync),
    /// but still implements all the calling infastructure of the `Runnable` trait. This provides
    /// a way for legion consumers to leverage the `System` construction and type-handling of
    /// this build for thread local systems which cannot leave the main initializating thread.
    pub fn build_thread_local<F>(self, run_fn: F) -> Box<dyn Runnable>
    where
        <R as ConsFlatten>::Output: ResourceSet + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: FnMut(
                &mut CommandBuffer,
                &mut PreparedWorld,
                &mut <<R as ConsFlatten>::Output as ResourceSet>::PreparedResources,
                &mut <<Q as ConsFlatten>::Output as QuerySet>::PreparedQueries,
            ) + 'static,
    {
        let disposable = SystemDisposableFnMut(run_fn, Default::default());
        Box::new(System {
            name: self.name,
            run_fn: AtomicRefCell::new(disposable),
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

    /// Builds a system which is considered `disposable`, which also means it carries an independent
    /// state object. This type of system construction allows for systems which may need to
    /// dispose of resources at the end of the process, which you cannot do from only `FnMut` capture
    /// variables.
    ///
    /// This variant of the function is like the `build_thread_local` function, allowing for constructing
    /// a thread local system which is !Send and !Sync.
    pub fn build_thread_local_disposable<S, F, D>(
        self,
        initial_state: S,
        run_fn: F,
        dispose_fn: D,
    ) -> Box<dyn Runnable>
    where
        S: 'static,
        <R as ConsFlatten>::Output: ResourceSet + Send + Sync,
        <Q as ConsFlatten>::Output: QuerySet,
        F: FnMut(
                &mut S,
                &mut CommandBuffer,
                &mut PreparedWorld,
                &mut <<R as ConsFlatten>::Output as ResourceSet>::PreparedResources,
                &mut <<Q as ConsFlatten>::Output as QuerySet>::PreparedQueries,
            ) + 'static,
        D: FnOnce(S, &mut World) + 'static,
    {
        let disposable = SystemDisposableState(
            run_fn,
            dispose_fn,
            StateWrapper(initial_state),
            Default::default(),
        );

        Box::new(System {
            name: self.name,
            run_fn: AtomicRefCell::new(disposable),
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
    use crate::schedule::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);

    #[derive(Default)]
    struct TestResource(pub i32);
    #[derive(Default)]
    struct TestResourceTwo(pub i32);
    #[derive(Default)]
    struct TestResourceThree(pub i32);
    #[derive(Default)]
    struct TestResourceFour(pub i32);

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TestComp(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TestCompTwo(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TestCompThree(f32, f32, f32);

    #[test]
    fn builder_schedule_execute() {
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();
        world.resources.insert(TestResource(123));
        world.resources.insert(TestResourceTwo(123));

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
            TestSystemFour,
        }

        let runs = Arc::new(Mutex::new(Vec::new()));

        let system_one_runs = runs.clone();
        let system_one = SystemBuilder::<()>::new("TestSystem1")
            .read_resource::<TestResource>()
            .with_query(Read::<Pos>::query())
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _world, _resource, _queries| {
                log::trace!("system_one");
                system_one_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemOne);
            });

        let system_two_runs = runs.clone();
        let system_two = SystemBuilder::<()>::new("TestSystem2")
            .write_resource::<TestResourceTwo>()
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _world, _resource, _queries| {
                log::trace!("system_two");
                system_two_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemTwo);
            });

        let system_three_runs = runs.clone();
        let system_three = SystemBuilder::<()>::new("TestSystem3")
            .read_resource::<TestResourceTwo>()
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _world, _resource, _queries| {
                log::trace!("system_three");
                system_three_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemThree);
            });
        let system_four_runs = runs.clone();
        let system_four = SystemBuilder::<()>::new("TestSystem4")
            .write_resource::<TestResourceTwo>()
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _world, _resource, _queries| {
                log::trace!("system_four");
                system_four_runs
                    .lock()
                    .unwrap()
                    .push(TestSystems::TestSystemFour);
            });

        let order = vec![
            TestSystems::TestSystemOne,
            TestSystems::TestSystemTwo,
            TestSystems::TestSystemThree,
            TestSystems::TestSystemFour,
        ];

        let mut systems = vec![system_one, system_two, system_three, system_four];

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(8)
            .build()
            .unwrap();

        let mut executor = StageExecutor::new(&mut systems, &pool);
        executor.execute(&mut world);

        assert_eq!(*(runs.lock().unwrap()), order);
    }

    #[test]
    fn builder_create_and_execute() {
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();
        world.resources.insert(TestResource(123));

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
            .read_resource::<TestResource>()
            .with_query(Read::<Pos>::query())
            .with_query(Read::<Vel>::query())
            .build(move |_commands, _world, resource, queries| {
                assert_eq!(resource.0, 123);
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
        system.run(&world);
    }

    #[test]
    fn fnmut_stateful_system_test() {
        let _ = env_logger::builder().is_test(true).try_init();

        let universe = Universe::new();
        let mut world = universe.create_world();
        world.resources.insert(TestResource(123));

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

        let mut state = 0;
        let mut system = SystemBuilder::<()>::new("TestSystem")
            .read_resource::<TestResource>()
            .with_query(Read::<Pos>::query())
            .with_query(Read::<Vel>::query())
            .build(move |_, _, _, _| {
                state += 1;
            });

        system.prepare(&world);
        system.run(&world);
    }
}
