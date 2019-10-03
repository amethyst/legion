use crate::borrow::{AtomicRefCell, Exclusive, Ref, RefMapMut, Shared};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    ops::{Deref, DerefMut},
};

pub enum ResourceAccessType {
    Read,
    Write,
}

pub trait Resource: 'static + Any + Send + Sync {}
impl<T> Resource for T where T: 'static + Any + Send + Sync {}

pub trait AccessType {
    fn access_type() -> ResourceAccessType;
    fn is_read() -> bool;
    fn is_write() -> bool;
}

pub struct Read<'a, T: Resource> {
    inner: Ref<'a, Shared<'a>, &'a T>,
}
impl<'a, T: Resource> Deref for Read<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.inner.deref() }
}
impl<'a, T: Resource> AccessType for Read<'a, T> {
    fn access_type() -> ResourceAccessType { ResourceAccessType::Read }
    fn is_read() -> bool { true }
    fn is_write() -> bool { false }
}

pub struct Write<'a, T: Resource> {
    inner: RefMapMut<'a, Exclusive<'a>, &'a mut T>,
}
impl<'a, T: Resource> Deref for Write<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.inner.deref() }
}

impl<'a, T: Resource> DerefMut for Write<'a, T> {
    fn deref_mut(&mut self) -> &mut T { self.inner.deref_mut() }
}

impl<'a, T: Resource> AccessType for Write<'a, T> {
    fn access_type() -> ResourceAccessType { ResourceAccessType::Write }
    fn is_read() -> bool { false }
    fn is_write() -> bool { true }
}

#[derive(Default)]
pub struct Resources {
    storage: HashMap<TypeId, AtomicRefCell<Box<dyn Resource>>>,
}

impl Resources {
    pub fn insert<T: Resource>(&mut self, value: T) {
        // We cant return the original inserted value becase we wrapped it in a box
        // I think anyways?
        self.storage
            .insert(TypeId::of::<T>(), AtomicRefCell::new(Box::new(value)));
    }

    pub fn get<T: Resource>(&self) -> Option<Read<'_, T>> {
        Some(Read {
            inner: self.storage.get(&TypeId::of::<T>())?.get().map(|v| unsafe {
                &*(v as *const std::boxed::Box<(dyn Resource + 'static)> as *const &T)
            }),
        })
    }

    pub fn get_mut<T: Resource>(&self) -> Option<Write<'_, T>> {
        Some(Write {
            inner: self
                .storage
                .get(&TypeId::of::<T>())?
                .get_mut()
                .map_into(|v| unsafe {
                    &mut *(v as *mut std::boxed::Box<(dyn Resource + 'static)> as *mut T)
                }),
        })
    }
}

pub trait Accessor: Send + Sync {
    type Output;

    fn reads(&self) -> Vec<TypeId>;
    fn writes(&self) -> Vec<TypeId>;
    fn fetch(resources: &Resources) -> Self::Output;
}

impl Accessor for () {
    type Output = ();

    fn reads(&self) -> Vec<TypeId> { vec![] }
    fn writes(&self) -> Vec<TypeId> { vec![] }
    fn fetch(_resources: &Resources) {}
}

macro_rules! impl_resource_tuple {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Resource ),*> Accessor for ($( $ty, )*)
        {
            type Output = ($( $ty, )*);

            fn reads(&self) -> Vec<TypeId> { vec![$( TypeId::of::<$ty>()),*] }
            fn writes(&self) -> Vec<TypeId> { vec![$( TypeId::of::<$ty>()),*] }
            fn fetch(_: &Resources) -> ($( $ty, )*) { unimplemented!() }
        }
    };
}

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
    }
}
