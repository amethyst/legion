//! Contains types related to defining shared resources which can be accessed inside systems.
//!
//! Use resources to share persistent data between systems or to provide a system with state
//! external to entities.

use crate::internals::{
    hash::ComponentTypeIdHasher,
    query::view::{read::Read, write::Write, ReadOnly},
};
use downcast_rs::{impl_downcast, Downcast};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    any::TypeId,
    collections::HashMap,
    fmt::{Display, Formatter},
    hash::{BuildHasherDefault, Hasher},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

/// Unique ID for a resource.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
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
    fn hash<H: Hasher>(&self, state: &mut H) { self.type_id.hash(state); }
}

impl Display for ResourceTypeId {
    #[cfg(debug_assertions)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.name) }

    #[cfg(not(debug_assertions))]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self.type_id) }
}

/// Blanket trait for resource types.
pub trait Resource: 'static + Downcast + Send + Sync {}
impl<T> Resource for T where T: 'static + Send + Sync {}
impl_downcast!(Resource);

/// Trait which is implemented for tuples of resources and singular resources. This abstracts
/// fetching resources to allow for ergonomic fetching.
///
/// # Example:
/// ```
///
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
///
/// ```
pub trait ResourceSet<'a>: Send + Sync {
    /// The resource reference returned during a fetch.
    type Result: 'a;

    /// Fetches all defined resources, without checking mutability.
    ///
    /// # Safety
    /// It is up to the end user to validate proper mutability rules across the resources being accessed.
    unsafe fn fetch_unchecked(resources: &'a Resources) -> Self::Result;

    /// Fetches all defined resources.
    fn fetch_mut(resources: &'a mut Resources) -> Self::Result {
        // safe because mutable borrow ensures exclusivity
        unsafe { Self::fetch_unchecked(resources) }
    }

    /// Fetches all defined resources.
    fn fetch(resources: &'a Resources) -> Self::Result
    where
        Self: ReadOnly,
    {
        unsafe { Self::fetch_unchecked(resources) }
    }
}

impl<'a> ResourceSet<'a> for () {
    type Result = ();

    unsafe fn fetch_unchecked(_: &Resources) {}
}

impl<'a, T: Resource> ResourceSet<'a> for Read<T> {
    type Result = Fetch<'a, T>;

    unsafe fn fetch_unchecked(resources: &'a Resources) -> Self::Result {
        resources
            .get::<T>()
            .unwrap_or_else(|| panic!("Failed to fetch resource!: {}", std::any::type_name::<T>()))
    }
}

impl<'a, T: Resource> ResourceSet<'a> for Write<T> {
    type Result = FetchMut<'a, T>;

    unsafe fn fetch_unchecked(resources: &'a Resources) -> Self::Result {
        resources
            .get_mut::<T>()
            .unwrap_or_else(|| panic!("Failed to fetch resource!: {}", std::any::type_name::<T>()))
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

            unsafe fn fetch_unchecked(resources: &'a Resources) -> Self::Result {
                ($( $ty::fetch_unchecked(resources), )*)
            }
        }
    };
}

#[cfg(feature = "extended-tuple-impls")]
resource_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
resource_tuple!(A, B, C, D, E, F, G, H);

/// Ergonomic wrapper type which contains a `Ref` type.
pub struct Fetch<'a, T: Resource> {
    inner: RwLockReadGuard<'a, Box<dyn Resource>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Resource> Deref for Fetch<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.downcast_ref::<T>().unwrap_or_else(|| {
            panic!(
                "Unable to downcast the resource!: {}",
                std::any::type_name::<T>()
            )
        })
    }
}

impl<'a, T: 'a + Resource + std::fmt::Debug> std::fmt::Debug for Fetch<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.deref())
    }
}

/// Ergonomic wrapper type which contains a `RefMut` type.
pub struct FetchMut<'a, T: Resource> {
    inner: RwLockWriteGuard<'a, Box<dyn Resource>>,
    _marker: PhantomData<T>,
}
impl<'a, T: 'a + Resource> Deref for FetchMut<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.downcast_ref::<T>().unwrap_or_else(|| {
            panic!(
                "Unable to downcast the resource!: {}",
                std::any::type_name::<T>()
            )
        })
    }
}

impl<'a, T: 'a + Resource> DerefMut for FetchMut<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.inner.downcast_mut::<T>().unwrap_or_else(|| {
            panic!(
                "Unable to downcast the resource!: {}",
                std::any::type_name::<T>()
            )
        })
    }
}

impl<'a, T: 'a + Resource + std::fmt::Debug> std::fmt::Debug for FetchMut<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.deref())
    }
}

/// Resources container. Shared resources stored here can be retrieved in systems.
#[derive(Default)]
pub struct Resources {
    storage: HashMap<
        ResourceTypeId,
        RwLock<Box<dyn Resource>>,
        BuildHasherDefault<ComponentTypeIdHasher>,
    >,
}

impl Resources {
    /// Returns `true` if type `T` exists in the store. Otherwise, returns `false`.
    pub fn contains<T: Resource>(&self) -> bool {
        self.storage.contains_key(&ResourceTypeId::of::<T>())
    }

    /// Inserts the instance of `T` into the store. If the type already exists, it will be silently
    /// overwritten. If you would like to retain the instance of the resource that already exists,
    /// call `remove` first to retrieve it.
    pub fn insert<T: Resource>(&mut self, value: T) {
        self.storage
            .insert(ResourceTypeId::of::<T>(), RwLock::new(Box::new(value)));
    }

    /// Removes the type `T` from this store if it exists.
    ///
    /// # Returns
    /// If the type `T` was stored, the inner instance of `T is returned. Otherwise, `None`.
    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        Some(
            *self
                .storage
                .remove(&ResourceTypeId::of::<T>())?
                .into_inner()
                .downcast::<T>()
                .ok()?,
        )
    }

    /// Retrieve an immutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    ///
    /// # Panics
    /// Panics if the resource is already borrowed mutably.
    pub fn get<T: Resource>(&self) -> Option<Fetch<'_, T>> {
        let type_id = &ResourceTypeId::of::<T>();
        Some(Fetch {
            inner: self.storage.get(type_id)?.try_read().unwrap_or_else(|| {
                panic!("resource {:?} already borrowed mutably elsewhere", type_id)
            }),
            _marker: PhantomData,
        })
    }

    /// Retrieve a mutable reference to  `T` from the store if it exists. Otherwise, return `None`.
    pub fn get_mut<T: Resource>(&self) -> Option<FetchMut<'_, T>> {
        let type_id = &ResourceTypeId::of::<T>();
        Some(FetchMut {
            inner: self
                .storage
                .get(type_id)?
                .try_write()
                .unwrap_or_else(|| panic!("resource {:?} already borrowed elsewhere", type_id)),
            _marker: PhantomData,
        })
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the closure `f` is called to construct the object and it is then inserted into the store.
    pub fn get_or_insert_with<T: Resource, F: FnOnce() -> T>(&mut self, f: F) -> Fetch<'_, T> {
        self.get_or_insert((f)())
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the closure `f` is called to construct the object and it is then inserted into the store.
    pub fn get_mut_or_insert_with<T: Resource, F: FnOnce() -> T>(
        &mut self,
        f: F,
    ) -> FetchMut<'_, T> {
        self.get_mut_or_insert((f)())
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the provided value is inserted and then a reference to it is returned.
    pub fn get_or_insert<T: Resource>(&mut self, value: T) -> Fetch<'_, T> {
        let type_id = ResourceTypeId::of::<T>();
        Fetch {
            inner: self
                .storage
                .entry(type_id)
                .or_insert_with(|| RwLock::new(Box::new(value)))
                .try_read()
                .unwrap_or_else(|| {
                    panic!("resource {:?} already borrowed mutably elsewhere", type_id)
                }),
            _marker: Default::default(),
        }
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the provided value is inserted and then a reference to it is returned.
    pub fn get_mut_or_insert<T: Resource>(&mut self, value: T) -> FetchMut<'_, T> {
        let type_id = ResourceTypeId::of::<T>();
        FetchMut {
            inner: self
                .storage
                .entry(type_id)
                .or_insert_with(|| RwLock::new(Box::new(value)))
                .try_write()
                .unwrap_or_else(|| {
                    panic!("resource {:?} already borrowed mutably elsewhere", type_id)
                }),
            _marker: Default::default(),
        }
    }

    /// Attempts to retrieve an immutable reference to `T` from the store. If it does not exist,
    /// the default constructor for `T` is called.
    ///
    /// `T` must implement `Default` for this method.
    pub fn get_or_default<T: Resource + Default>(&mut self) -> Fetch<'_, T> {
        let type_id = ResourceTypeId::of::<T>();
        Fetch {
            inner: self
                .storage
                .entry(type_id)
                .or_insert_with(|| RwLock::new(Box::new(T::default())))
                .try_read()
                .unwrap_or_else(|| {
                    panic!("resource {:?} already borrowed mutably elsewhere", type_id)
                }),
            _marker: Default::default(),
        }
    }

    /// Attempts to retrieve a mutable reference to `T` from the store. If it does not exist,
    /// the default constructor for `T` is called.
    ///
    /// `T` must implement `Default` for this method.
    pub fn get_mut_or_default<T: Resource + Default>(&mut self) -> FetchMut<'_, T> {
        let type_id = ResourceTypeId::of::<T>();
        FetchMut {
            inner: self
                .storage
                .entry(type_id)
                .or_insert_with(|| RwLock::new(Box::new(T::default())))
                .try_write()
                .unwrap_or_else(|| {
                    panic!("resource {:?} already borrowed mutably elsewhere", type_id)
                }),
            _marker: Default::default(),
        }
    }

    /// Performs merging of two resource storages, which occurs during a world merge.
    /// This merge will retain any already-existant resources in the local world, while moving any
    /// new resources from the source world into this one, consuming the resources.
    pub fn merge(&mut self, mut other: Resources) {
        // Merge resources, retaining our local ones but moving in any non-existant ones
        for resource in other.storage.drain() {
            self.storage.entry(resource.0).or_insert(resource.1);
        }
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

        let mut resources = Resources::default();
        resources.insert(TestOne {
            value: "one".to_string(),
        });

        resources.insert(TestTwo {
            value: "two".to_string(),
        });

        assert_eq!(resources.get::<TestOne>().unwrap().value, "one");
        assert_eq!(resources.get::<TestTwo>().unwrap().value, "two");

        // test re-ownership
        let owned = resources.remove::<TestTwo>();
        assert_eq!(owned.unwrap().value, "two")
    }
}
