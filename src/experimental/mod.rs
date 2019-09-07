#![allow(dead_code)]

pub mod borrow;
pub mod entity;
pub mod filter;
pub mod query;
pub mod storage;
pub mod world;

pub mod prelude {
    pub use crate::experimental::entity::Entity;
    pub use crate::experimental::filter::filter_fns::*;
    pub use crate::experimental::query::{IntoQuery, Query, Read, Tagged, Write};
    pub use crate::experimental::world::{Universe, World};
}
