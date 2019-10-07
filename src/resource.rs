use crate::borrow::{AtomicRefCell, Exclusive, Ref, RefMapMut, Shared};
use derivative::Derivative;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

pub enum ResourceAccessType {
    Read,
    Write,
}

#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct ReadWrapper<'a, T> {
    _marker: PhantomData<T>,
}

impl<'a, T: Resource> Accessor for ReadWrapper<'a, T> {
    type Output = Read<'a, T>;

    fn reads() -> Vec<TypeId> { vec![] }
    fn writes() -> Vec<TypeId> { vec![] }
    fn fetch<'a>(_: &'a Resources) -> Self::Output { unimplemented!() }
}

#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct WriteWrapper<T> {
    _marker: PhantomData<T>,
}
impl<T: Resource> Accessor for WriteWrapper<T> {
    type Output = Self;

    fn reads() -> Vec<TypeId> { vec![] }
    fn writes() -> Vec<TypeId> { vec![] }
    fn fetch(_: &Resources) -> Self::Output { unimplemented!() }
}

pub trait Resource: 'static + Any + Send + Sync {}
impl<T> Resource for T where T: 'static + Any + Send + Sync {}

pub trait AccessType {
    fn access_type() -> ResourceAccessType;
    fn is_read() -> bool;
    fn is_write() -> bool;
}

pub trait Accessor: Send + Sync {
    type Output: Accessor;

    fn reads() -> Vec<TypeId>;
    fn writes() -> Vec<TypeId>;
    fn fetch(resources: &Resources) -> Self::Output;
}

pub struct Read<'a, T: 'a + Resource> {
    inner: Ref<'a, Shared<'a>, &'a T>,
}
impl<'a, T: Resource> Deref for Read<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.inner.deref() }
}
impl<'a, T: 'a + Resource> AccessType for Read<'a, T> {
    fn access_type() -> ResourceAccessType { ResourceAccessType::Read }
    fn is_read() -> bool { true }
    fn is_write() -> bool { false }
}
impl<'a, T: 'a + Resource> Accessor for Read<'a, T> {
    type Output = Read<'a, T>;

    fn reads() -> Vec<TypeId> { vec![] }
    fn writes() -> Vec<TypeId> { vec![] }
    fn fetch(_: &Resources) -> Self::Output { unimplemented!() }
}

pub struct Write<'a, T: Resource> {
    inner: RefMapMut<'a, Exclusive<'a>, &'a mut T>,
}
impl<'a, T: 'a + Resource> Deref for Write<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.inner.deref() }
}

impl<'a, T: 'a + Resource> DerefMut for Write<'a, T> {
    fn deref_mut(&mut self) -> &mut T { self.inner.deref_mut() }
}

impl<'a, T: 'a + Resource> AccessType for Write<'a, T> {
    fn access_type() -> ResourceAccessType { ResourceAccessType::Write }
    fn is_read() -> bool { false }
    fn is_write() -> bool { true }
}
impl<'a, T: Resource> Accessor for Write<'a, T> {
    type Output = Self;

    fn reads() -> Vec<TypeId> { vec![] }
    fn writes() -> Vec<TypeId> { vec![] }
    fn fetch(_: &Resources) -> Self::Output { unimplemented!() }
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

impl Accessor for () {
    type Output = ();

    fn reads() -> Vec<TypeId> { vec![] }
    fn writes() -> Vec<TypeId> { vec![] }
    fn fetch(_: &Resources) {}
}

macro_rules! impl_resource_tuple {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Resource ),*> Accessor for ($( $ty, )*)
        {
            type Output = Self;
            fn reads() -> Vec<TypeId> { vec![$( TypeId::of::<$ty>()),*] }
            fn writes() -> Vec<TypeId> { vec![$( TypeId::of::<$ty>()),*] }
            fn fetch(_: &Resources) -> Self::Output { unimplemented!() }
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
    }
}
