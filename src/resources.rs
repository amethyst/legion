use crate::borrow::AtomicRefCell;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
};

pub enum ResourceAccessType {
    Read,
    Write,
}

pub trait Resource: Any + Send + Sync {}
impl<T> Resource for T where T: Any + Send + Sync {}

#[derive(Default)]
pub struct Resources(HashMap<TypeId, AtomicRefCell<Box<dyn Resource>>>);
