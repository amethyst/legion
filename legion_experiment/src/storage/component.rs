use super::{packed::PackedStorage, ComponentStorage};
use std::any::TypeId;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ComponentTypeId(TypeId);

impl ComponentTypeId {
    pub fn of<T: Component>() -> Self {
        Self(TypeId::of::<T>())
    }
}

pub trait Component: 'static + Sized + Send + Sync {
    type Storage: for<'a> ComponentStorage<'a, Self>;
}

impl<T: 'static + Sized + Send + Sync> Component for T {
    type Storage = PackedStorage<T>;
}
