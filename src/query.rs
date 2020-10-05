//! Queries provide efficient iteration and filtering of entity components in a world.
//!
//! Queries are defined by two parts; "views" and "filters".
//! Views declare what data you want to access, and how you want to access it.
//! Filters decide which entities are to be included in the results.
//!
//! To construct a query, we declare our view, and then call `::query()` to convert it into
//! a query with an initial filter which selects entities with all of the component types
//! requested by the view.
//!
//! View types include [Entity](../world/struct.Entity.html), [Read](struct.Read.html),
//! [Write](struct.Write.html), [TryRead](struct.TryRead.html) and [TryWrite](struct.TryWrite.html).
//!
//! ```
//! # use legion::*;
//! # struct Position;
//! # struct Orientation;
//! // a view can be a single view type
//! let mut query = <&Position>::query();
//!
//! // or a tuple of views
//! let mut query = <(&Position, &mut Orientation)>::query();
//! ```
//!
//! You can attach additional filters to a query to further refine which entities you want to access.
//!
//! ```
//! # use legion::*;
//! # struct Position;
//! # struct Orientation;
//! # struct Model;
//! # struct Static;
//!
//! // filters can be combined with boolean operators
//! let mut query = <(&Position, &mut Orientation)>::query()
//!     .filter(!component::<Static>() | !component::<Model>());
//! ```
//!
//! Once you have a query, you can use it to pull data out of a world. At its core, a query
//! allows you to iterate over [chunks](struct.ChunkView.html). Each chunk contains a set of
//! entities which all have exactly the same component types attached, and the chunk provides
//! access to slices of each component. A single index in each slice in a chunk contains the
//! component for the same entity.
//!
//! ```
//! # use legion::*;
//! # struct Position;
//! # struct Orientation;
//! # let mut world = World::default();
//! let mut query = <(&Position, &mut Orientation)>::query();
//! for mut chunk in query.iter_chunks_mut(&mut world) {
//!     // we can access information about the archetype (shape/component layout) of the entities
//!     println!(
//!         "the entities in the chunk have {:?} components",
//!         chunk.archetype().layout().component_types(),
//!     );
//!
//!     // we can iterate through a tuple of component references
//!     for (position, orientation) in chunk {
//!         // position is a `&Position`
//!         // orientation is a `&mut Orientation`
//!         // they are both attached to the same entity
//!     }
//! }
//! ```
//!
//! There are convenience functions on query which will flatten this loop for us, giving
//! direct access to the entities.
//!
//! ```
//! # use legion::*;
//! # struct Position;
//! # struct Orientation;
//! # let mut world = World::default();
//! let mut query = <(&Position, &mut Orientation)>::query();
//! for (position, orientation) in query.iter_mut(&mut world) {
//!     // position is a `&Position`
//!     // orientation is a `&mut Orientation`
//!     // they are both attached to the same entity
//! }
//! ```

pub use crate::internals::query::{
    filter::{
        and::And,
        any::Any,
        component::ComponentFilter,
        filter_fns::{any, component, maybe_changed, passthrough},
        maybe_changed::ComponentChangedFilter,
        not::Not,
        or::Or,
        passthrough::Passthrough,
        try_component::TryComponentFilter,
        DynamicFilter, EntityFilter, FilterResult, GroupMatcher, LayoutFilter,
    },
    view::{
        read::Read, try_read::TryRead, try_write::TryWrite, write::Write, DefaultFilter, Fetch,
        IntoIndexableIter, ReadOnly, View,
    },
    ChunkIter, ChunkView, IntoQuery, Query,
};

#[cfg(feature = "parallel")]
pub use crate::internals::query::par_iter::{Iter, ParChunkIter};
