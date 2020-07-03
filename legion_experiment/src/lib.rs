pub mod cons;
pub mod entity;
pub mod entry;
pub mod event;
mod hash;
mod insert;
mod iter;
pub mod permissions;
pub mod query;
#[cfg(feature = "serialize")]
pub mod serialize;
pub mod storage;
pub mod subworld;
pub mod systems;
pub mod world;

// re-export most common types
pub use crate::{
    entity::Entity,
    event::Event,
    insert::IntoSoa,
    query::{filter::filter_fns::*, view::Fetch, IntoQuery, Query},
    systems::{
        resources::Resources,
        schedule::{Executor, Schedule},
        system::SystemBuilder,
    },
    world::{ConflictPolicy, Duplicate, EntityPolicy, Move, Universe, World},
};

#[cfg(feature = "serialize")]
pub use crate::serialize::Registry;
