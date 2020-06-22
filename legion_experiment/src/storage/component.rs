use super::{packed::PackedStorage, ComponentStorage};
use std::{
    any::TypeId,
    fmt::{Display, Formatter},
    hash::Hasher,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
pub struct ComponentTypeId {
    type_id: TypeId,
    #[cfg(debug_assertions)]
    name: &'static str,
}

impl ComponentTypeId {
    pub fn of<T: Component>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            #[cfg(debug_assertions)]
            name: std::any::type_name::<T>(),
        }
    }
}

impl std::hash::Hash for ComponentTypeId {
    fn hash<H: Hasher>(&self, state: &mut H) { self.type_id.hash(state); }
}

impl Display for ComponentTypeId {
    #[cfg(debug_assertions)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.name) }

    #[cfg(not(debug_assertions))]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self.type_id) }
}

pub trait Component: 'static + Sized + Send + Sync {
    type Storage: for<'a> ComponentStorage<'a, Self>;
}

impl<T: 'static + Sized + Send + Sync> Component for T {
    type Storage = PackedStorage<T>;
}
