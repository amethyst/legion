//! Contains types related to entity components.

use super::{packed::PackedStorage, ComponentStorage};
use std::{
    any::TypeId,
    fmt::{Display, Formatter},
    hash::Hasher,
};

/// A unique ID for a component type.
#[derive(Copy, Clone, Debug, Eq, PartialOrd, Ord)]
pub struct ComponentTypeId {
    pub(crate) type_id: TypeId,
    #[cfg(debug_assertions)]
    name: &'static str,
}

impl ComponentTypeId {
    /// Constructs the component type ID for the given component type.
    pub fn of<T: Component>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            #[cfg(debug_assertions)]
            name: std::any::type_name::<T>(),
        }
    }

    /// Returns the internal TypeID of the component.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }
}

impl std::hash::Hash for ComponentTypeId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl PartialEq for ComponentTypeId {
    fn eq(&self, other: &Self) -> bool {
        self.type_id.eq(&other.type_id)
    }
}

impl Display for ComponentTypeId {
    #[cfg(debug_assertions)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }

    #[cfg(not(debug_assertions))]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.type_id)
    }
}

/// A marker trait for all types which can be attached to an entity.
///
/// This trait has a blanket impl for all applicable types.
pub trait Component: 'static + Sized + Send + Sync {
    /// The storage type required to hold all instances of this component in a world.
    type Storage: for<'a> ComponentStorage<'a, Self>;
}

impl<T: 'static + Sized + Send + Sync> Component for T {
    type Storage = PackedStorage<T>;
}
