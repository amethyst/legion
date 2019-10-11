use crate::borrow::{AtomicRefCell, Exclusive, Ref, RefMut, Shared};
use crate::query::{Read, Write};
use derivative::Derivative;
use mopa::Any;
use std::{
    any::TypeId,
    collections::HashMap,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

pub trait ResourceSet: Send + Sync {
    type PreparedResources;

    fn fetch<'a>(&self, resources: &'a Resources) -> Self::PreparedResources;
}

pub trait Resource: 'static + Any + Send + Sync {}
impl<T> Resource for T where T: 'static + Any + Send + Sync {}

mod __resource_mopafy_scope {
    #![allow(clippy::all)]

    use mopa::mopafy;

    use super::Resource;

    mopafy!(Resource);
}

pub struct PreparedRead<T: Resource> {
    resource: *const T,
}
impl<T: Resource> PreparedRead<T> {
    pub(crate) unsafe fn new(resource: *const T) -> Self { Self { resource } }
}
impl<T: Resource> Deref for PreparedRead<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { unsafe { &*self.resource } }
}
unsafe impl<T: Resource> Send for PreparedRead<T> {}
unsafe impl<T: Resource> Sync for PreparedRead<T> {}

pub struct PreparedWrite<T: Resource> {
    resource: *mut T,
}
impl<T: Resource> Deref for PreparedWrite<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { unsafe { &*self.resource } }
}

impl<T: Resource> DerefMut for PreparedWrite<T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.resource } }
}
impl<T: Resource> PreparedWrite<T> {
    pub(crate) unsafe fn new(resource: *mut T) -> Self { Self { resource } }
}
unsafe impl<T: Resource> Send for PreparedWrite<T> {}
unsafe impl<T: Resource> Sync for PreparedWrite<T> {}

pub struct Fetch<'a, T: 'a + Resource> {
    inner: Ref<'a, Shared<'a>, Box<dyn Resource>>,
    _marker: PhantomData<T>,
}
impl<'a, T: Resource> Deref for Fetch<'a, T> {
    type Target = T;

    //  #[cfg(debug_assertions)]
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.downcast_ref::<T>().expect(&format!(
            "Unable to downcast the resource!: {}",
            std::any::type_name::<T>()
        ))
    }

    //  #[cfg(not(debug_assertions))]
    //  #[inline]
    //  fn deref(&self) -> &Self::Target { unsafe { self.inner.downcast_ref_unchecked::<T>() } }
}

pub struct FetchMut<'a, T: Resource> {
    inner: RefMut<'a, Exclusive<'a>, Box<dyn Resource>>,
    _marker: PhantomData<T>,
}
impl<'a, T: 'a + Resource> Deref for FetchMut<'a, T> {
    type Target = T;

    //  #[cfg(debug_assertions)]
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.downcast_ref::<T>().expect(&format!(
            "Unable to downcast the resource!: {}",
            std::any::type_name::<T>()
        ))
    }

    //    #[cfg(not(debug_assertions))]
    //    #[inline]
    //    fn deref(&self) -> &Self::Target { unsafe { self.inner.downcast_ref_unchecked::<T>() } }
}

impl<'a, T: 'a + Resource> DerefMut for FetchMut<'a, T> {
    //    #[cfg(debug_assertions)]
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.inner.downcast_mut::<T>().expect(&format!(
            "Unable to downcast the resource!: {}",
            std::any::type_name::<T>()
        ))
    }

    //   #[cfg(not(debug_assertions))]
    //   #[inline]
    //   fn deref_mut(&mut self) -> &mut T { unsafe { self.inner.downcast_mut_unchecked::<T>() } }
}

#[derive(Default)]
pub struct Resources {
    storage: HashMap<TypeId, AtomicRefCell<Box<dyn Resource>>>,
}

impl Resources {
    pub fn contains<T: Resource>(&self) -> bool { self.storage.contains_key(&TypeId::of::<T>()) }

    pub fn insert<T: Resource>(&mut self, value: T) {
        self.storage
            .insert(TypeId::of::<T>(), AtomicRefCell::new(Box::new(value)));
    }

    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        Some(
            *self
                .storage
                .remove(&TypeId::of::<T>())?
                .into_inner()
                .downcast::<T>()
                .ok()?,
        )
    }

    pub fn get<T: Resource>(&self) -> Option<Fetch<'_, T>> {
        Some(Fetch {
            inner: self.storage.get(&TypeId::of::<T>())?.get(),
            _marker: Default::default(),
        })
    }

    pub fn get_mut<T: Resource>(&self) -> Option<FetchMut<'_, T>> {
        Some(FetchMut {
            inner: self.storage.get(&TypeId::of::<T>())?.get_mut(),
            _marker: Default::default(),
        })
    }
}

impl ResourceSet for () {
    type PreparedResources = ();

    fn fetch(&self, _: &Resources) {}
}

impl<T: Resource> ResourceSet for Read<T> {
    type PreparedResources = PreparedRead<T>;

    fn fetch(&self, resources: &Resources) -> Self::PreparedResources {
        let resource = resources.get::<T>().expect(&format!(
            "Failed to fetch resource: {}",
            std::any::type_name::<T>()
        ));
        unsafe { PreparedRead::new(resource.deref() as *const T) }
    }
}
impl<T: Resource> ResourceSet for Write<T> {
    type PreparedResources = PreparedWrite<T>;

    fn fetch(&self, resources: &Resources) -> Self::PreparedResources {
        let mut resource = resources.get_mut::<T>().expect(&format!(
            "Failed to fetch resource: {}",
            std::any::type_name::<T>()
        ));
        unsafe { PreparedWrite::new(resource.deref_mut() as *mut T) }
    }
}

macro_rules! impl_resource_tuple {
    ( $( $ty: ident ),* ) => {
        #[allow(unused_parens, non_snake_case)]
        impl<$( $ty: ResourceSet ),*> ResourceSet for ($( $ty, )*)
        {
            type PreparedResources = ($( $ty::PreparedResources, )*);

            fn fetch(&self, resources: &Resources) -> Self::PreparedResources {
                let ($($ty,)*) = self;
                ($( $ty.fetch(resources), )*)
             }
        }
    };
}
//($( $ty, )*)

impl_resource_tuple!(A);
impl_resource_tuple!(A, B);
impl_resource_tuple!(A, B, C);
impl_resource_tuple!(A, B, C, D);
impl_resource_tuple!(A, B, C, D, E);
impl_resource_tuple!(A, B, C, D, E, F);
impl_resource_tuple!(A, B, C, D, E, F, G);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_read_write_test() {
        let _ = env_logger::builder().is_test(true).try_init();

        struct TestOne {
            value: String,
        }

        struct TestTwo {
            value: String,
        }

        let mut resources = Resources::default();
        resources.insert(TestOne {
            value: "poop".to_string(),
        });

        resources.insert(TestTwo {
            value: "balls".to_string(),
        });

        assert_eq!(resources.get::<TestOne>().unwrap().value, "poop");
        assert_eq!(resources.get::<TestTwo>().unwrap().value, "balls");

        // test re-ownership
        let owned = resources.remove::<TestTwo>();
        assert_eq!(owned.unwrap().value, "balls")
    }
}
