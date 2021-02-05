//! Contains types related to defining shared resources which can be accessed inside systems.
//!
//! Use resources to share persistent data between systems or to provide a system with state
//! external to entities.

use std::{
    any::TypeId,
    collections::{hash_map::Entry, HashMap},
    fmt::{Display, Formatter},
    hash::{BuildHasherDefault, Hasher},
    marker::PhantomData,
};

use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use downcast_rs::{impl_downcast, Downcast};

use crate::internals::{
    hash::ComponentTypeIdHasher,
    query::view::{read::Read, write::Write, ReadOnly},
};

/// Unique ID for a resource.
#[derive(Copy, Clone, Debug, Eq, PartialOrd, Ord)]
pub struct ResourceTypeId {
    type_id: TypeId,
    #[cfg(debug_assertions)]
    name: &'static str,
}

impl ResourceTypeId {
    /// Returns the resource type ID of the given resource type.
    pub fn of<T: Resource>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            #[cfg(debug_assertions)]
            name: std::any::type_name::<T>(),
        }
    }
}

impl std::hash::Hash for ResourceTypeId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl PartialEq for ResourceTypeId {
    fn eq(&self, other: &Self) -> bool {
        self.type_id.eq(&other.type_id)
    }
}

impl Display for ResourceTypeId {
    #[cfg(debug_assertions)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }

    #[cfg(not(debug_assertions))]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.type_id)
    }
}

/// Blanket trait for resource types.
pub trait Resource: 'static + Downcast {}
impl<T> Resource for T where T: 'static {}
impl_downcast!(Resource);

/// Trait which is implemented for tuples of resources and singular resources. This abstracts
/// fetching resources to allow for ergonomic fetching.
///
/// # Example:
/// ```
/// struct TypeA(usize);
/// struct TypeB(usize);
///
/// # use legion::*;
/// # use legion::systems::ResourceSet;
/// let mut resources = Resources::default();
/// resources.insert(TypeA(55));
/// resources.insert(TypeB(12));
///
/// {
///     let (a, mut b) = <(Read<TypeA>, Write<TypeB>)>::fetch_mut(&mut resources);
///     assert_ne!(a.0, b.0);
///     b.0 = a.0;
/// }
///
/// {
///     let (a, b) = <(Read<TypeA>, Read<TypeB>)>::fetch(&resources);
///     assert_eq!(a.0, b.0);
/// }
/// ```
pub trait ResourceSet<'a> {
    /// The resource reference returned during a fetch.
    type Result: 'a;

    /// Fetches all defined resources, without checking mutability.
    ///
    /// # Safety
    /// It is up to the end user to validate proper mutability rules across the resources being accessed.
    unsafe fn fetch_unchecked(resources: &'a UnsafeResources) -> Self::Result;

    /// Fetches all defined resources.
    fn fetch_mut(resources: &'a mut Resources) -> Self::Result {
        // safe because mutable borrow ensures exclusivity
        unsafe { Self::fetch_unchecked(&resources.internal) }
    }

    /// Fetches all defined resources.
    fn fetch(resources: &'a Resources) -> Self::Result
    where
        Self: ReadOnly,
    {
        unsafe { Self::fetch_unchecked(&resources.internal) }
    }
}

impl<'a> ResourceSet<'a> for () {
    type Result = ();

    unsafe fn fetch_unchecked(_: &UnsafeResources) -> Self::Result {}
}

impl<'a, T: Resource> ResourceSet<'a> for Read<T> {
    type Result = AtomicRef<'a, T>;

    unsafe fn fetch_unchecked(resources: &'a UnsafeResources) -> Self::Result {
        let type_id = &ResourceTypeId::of::<T>();
        resources
            .get(&type_id)
            .map(|x| x.get::<T>())
            .expect("resource does not exist")
    }
}

impl<'a, T: Resource> ResourceSet<'a> for Write<T> {
    type Result = AtomicRefMut<'a, T>;

    unsafe fn fetch_unchecked(resources: &'a UnsafeResources) -> Self::Result {
        let type_id = &ResourceTypeId::of::<T>();
        resources
            .get(&type_id)
            .map(|x| x.get_mut::<T>())
            .expect("resource does not exist")
    }
}

macro_rules! resource_tuple {
    ($head_ty:ident) => {
        impl_resource_tuple!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_resource_tuple!($head_ty, $( $tail_ty ),*);
        resource_tuple!($( $tail_ty ),*);
    );
}

macro_rules! impl_resource_tuple {
    ( $( $ty: ident ),* ) => {
        #[allow(unused_parens, non_snake_case)]
        impl<'a, $( $ty: ResourceSet<'a> ),*> ResourceSet<'a> for ($( $ty, )*)
        {
            type Result = ($( $ty::Result, )*);

            unsafe fn fetch_unchecked(resources: &'a UnsafeResources) -> Self::Result {
                ($( $ty::fetch_unchecked(resources), )*)
            }
        }
    };
}

#[cfg(feature = "extended-tuple-impls")]
resource_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
resource_tuple!(A, B, C, D, E, F, G, H);

pub struct ResourceCell {
    data: AtomicRefCell<Box<dyn Resource>>,
}

impl ResourceCell {
    fn new(resource: Box<dyn Resource>) -> Self {
        Self {
            data: AtomicRefCell::new(resource),
        }
    }

    fn into_inner(self) -> Box<dyn Resource> {
        self.data.into_inner()
    }

    /// # Safety
    /// Types which are !Sync should only be retrieved on the thread which owns the resource
    /// collection.
    pub fn get<T: Resource>(&self) -> AtomicRef<T> {
        let borrow = self.data.borrow();
        AtomicRef::map(borrow, |inner| inner.downcast_ref::<T>().unwrap())
    }

    /// # Safety
    /// Types which are !Send should only be retrieved on the thread which owns the resource
    /// collection.
    pub fn get_mut<T: Resource>(&self) -> AtomicRefMut<T> {
        let borrow = self.data.borrow_mut(); // panics if this is borrowed already

        AtomicRefMut::map(borrow, |inner| inner.downcast_mut::<T>().unwrap())
    }
}

/// A container for resources which performs runtime borrow checking
/// but _does not_ ensure that `!Sync` resources aren't accessed across threads.
#[derive(Default)]
pub struct UnsafeResources {
    map: HashMap<ResourceTypeId, ResourceCell, BuildHasherDefault<ComponentTypeIdHasher>>,
}

unsafe impl Send for UnsafeResources {}
unsafe impl Sync for UnsafeResources {}

impl UnsafeResources {
    fn contains(&self, type_id: &ResourceTypeId) -> bool {
        self.map.contains_key(type_id)
    }

    /// # Safety
    /// Resources which are `!Sync` or `!Send` must be retrieved or inserted only on the main thread.
    unsafe fn entry(&mut self, type_id: ResourceTypeId) -> Entry<ResourceTypeId, ResourceCell> {
        self.map.entry(type_id)
    }

    /// # Safety
    /// Resources which are `!Send` must be retrieved or inserted only on the main thread.
    unsafe fn insert<T: Resource>(&mut self, resource: T) {
        self.map.insert(
            ResourceTypeId::of::<T>(),
            ResourceCell::new(Box::new(resource)),
        );
    }

    /// # Safety
    /// Resources which are `!Send` must be retrieved or inserted only on the main thread.
    unsafe fn remove(&mut self, type_id: &ResourceTypeId) -> Option<Box<dyn Resource>> {
        self.map.remove(type_id).map(|cell| cell.into_inner())
    }

    fn get(&self, type_id: &ResourceTypeId) -> Option<&ResourceCell> {
        self.map.get(type_id)
    }

    /// # Safety
    /// Resources which are `!Sync` must be retrieved or inserted only on the main thread.
    unsafe fn merge(&mut self, mut other: Self) {
        // Merge resources, retaining our local ones but moving in any non-existant ones
        for resource in other.map.drain() {
            self.map.entry(resource.0).or_insert(resource.1);
        }
    }
}

/// Resources container. Shared resources stored here can be retrieved in systems.
#[derive(Default)]
pub struct Resources {
    internal: UnsafeResources,
    // marker to make `Resources` !Send and !Sync
    _not_send_sync: PhantomData<*const u8>,
}

impl Resources {
    pub(crate) fn internal(&self) -> &UnsafeResources {
        &self.internal
    }

    /// Creates an accessor to resources which are Send and Sync, which itself can be sent
    /// between threads.
    pub fn sync(&mut self) -> SyncResources {
        SyncResources {
            internal: &self.internal,
        }
    }

    /// Returns `true` if type `T` exists in the store. Otherwise, returns `false`.
    pub fn contains<T: Resource>(&self) -> bool {
        self.internal.contains(&ResourceTypeId::of::<T>())
    }

    /// Inserts the instance of `T` into the store. If the type already exists, it will be silently
    /// overwritten. If you would like to retain the instance of the resource that already exists,
    /// call `remove` first to retrieve it.
    pub fn insert<T: Resource>(&mut self, value: T) {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        unsafe {
            self.internal.insert(value);
        }
    }

    /// Removes the type `T` from this store if it exists.
    ///
    /// # Returns
    /// If the type `T` was stored, the inner instance of `T is returned. Otherwise, `None`.
    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        unsafe {
            let resource = self
                .internal
                .remove(&ResourceTypeId::of::<T>())?
                .downcast::<T>()
                .ok()?;
            Some(*resource)
        }
    }

    /// Retrieve an immutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    ///
    /// # Panics
    /// Panics if the resource is already borrowed mutably.
    pub fn get<T: Resource>(&self) -> Option<AtomicRef<T>> {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        let type_id = &ResourceTypeId::of::<T>();
        self.internal.get(&type_id).map(|x| x.get::<T>())
    }

    /// Retrieve a mutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    pub fn get_mut<T: Resource>(&self) -> Option<AtomicRefMut<T>> {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        let type_id = &ResourceTypeId::of::<T>();
        self.internal.get(&type_id).map(|x| x.get_mut::<T>())
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the closure `f` is called to construct the object and it is then inserted into the store.
    pub fn get_or_insert_with<T: Resource, F: FnOnce() -> T>(&mut self, f: F) -> AtomicRef<T> {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        let type_id = ResourceTypeId::of::<T>();
        unsafe {
            self.internal
                .entry(type_id)
                .or_insert_with(|| ResourceCell::new(Box::new((f)())))
                .get()
        }
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the closure `f` is called to construct the object and it is then inserted into the store.
    pub fn get_mut_or_insert_with<T: Resource, F: FnOnce() -> T>(
        &mut self,
        f: F,
    ) -> AtomicRefMut<T> {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        let type_id = ResourceTypeId::of::<T>();
        unsafe {
            self.internal
                .entry(type_id)
                .or_insert_with(|| ResourceCell::new(Box::new((f)())))
                .get_mut()
        }
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the provided value is inserted and then a reference to it is returned.
    pub fn get_or_insert<T: Resource>(&mut self, value: T) -> AtomicRef<T> {
        self.get_or_insert_with(|| value)
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the provided value is inserted and then a reference to it is returned.
    pub fn get_mut_or_insert<T: Resource>(&mut self, value: T) -> AtomicRefMut<T> {
        self.get_mut_or_insert_with(|| value)
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the default constructor for `T` is called.
    ///
    /// `T` must implement `Default` for this method.
    pub fn get_or_default<T: Resource + Default>(&mut self) -> AtomicRef<T> {
        self.get_or_insert_with(T::default)
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the default constructor for `T` is called.
    ///
    /// `T` must implement `Default` for this method.
    pub fn get_mut_or_default<T: Resource + Default>(&mut self) -> AtomicRefMut<T> {
        self.get_mut_or_insert_with(T::default)
    }

    /// Performs merging of two resource storages, which occurs during a world merge.
    /// This merge will retain any already-existant resources in the local world, while moving any
    /// new resources from the source world into this one, consuming the resources.
    pub fn merge(&mut self, other: Resources) {
        // safety:
        // this type is !Send and !Sync, and so can only be accessed from the thread which
        // owns the resources collection
        unsafe {
            self.internal.merge(other.internal);
        }
    }
}

/// A resource collection which is `Send` and `Sync`, but which only allows access to resources
/// which are `Sync`.
pub struct SyncResources<'a> {
    internal: &'a UnsafeResources,
}

impl<'a> SyncResources<'a> {
    /// Retrieve an immutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    ///
    /// # Panics
    /// Panics if the resource is already borrowed mutably.
    pub fn get<T: Resource + Sync>(&self) -> Option<AtomicRef<T>> {
        let type_id = &ResourceTypeId::of::<T>();
        self.internal.get(&type_id).map(|x| x.get::<T>())
    }

    /// Retrieve a mutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    pub fn get_mut<T: Resource + Send>(&self) -> Option<AtomicRefMut<T>> {
        let type_id = &ResourceTypeId::of::<T>();
        self.internal.get(&type_id).map(|x| x.get_mut::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_read_write_test() {
        struct TestOne {
            value: String,
        }

        struct TestTwo {
            value: String,
        }

        struct NotSync {
            ptr: *const u8,
        }

        let mut resources = Resources::default();
        resources.insert(TestOne {
            value: "one".to_string(),
        });

        resources.insert(TestTwo {
            value: "two".to_string(),
        });

        resources.insert(NotSync {
            ptr: std::ptr::null(),
        });

        assert_eq!(resources.get::<TestOne>().unwrap().value, "one");
        assert_eq!(resources.get::<TestTwo>().unwrap().value, "two");
        assert_eq!(resources.get::<NotSync>().unwrap().ptr, std::ptr::null());

        // test re-ownership
        let owned = resources.remove::<TestTwo>();
        assert_eq!(owned.unwrap().value, "two");
    }
}
