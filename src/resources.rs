use crate::borrow::AtomicRefCell;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

pub enum ResourceAccessType {
    Read,
    Write,
}

pub trait Resource: 'static + Any + Send + Sync {}
impl<T> Resource for T where T: 'static + Any + Send + Sync {}

#[derive(Default)]
pub struct Resources(HashMap<TypeId, AtomicRefCell<Box<dyn Resource>>>);

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
