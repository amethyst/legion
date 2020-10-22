//! A "packed archetype" storage model.
//!
//! Any combination of types of components can be attached to each entity
//! in a [world](../world/struct.World.html). Storing the (potentially
//! unique) set of component values for each entity in a manner which is
//! efficient to search and access is the responsibility of the ECS libary.
//!
//! Legion achieves this via the use of "archetypes". Archetypes are a
//! collection of entities who all have exactly the same set of component
//! types attached. By storing these together, we can perform filtering
//! operations at the archetype level without needing to ever inspect each
//! individual entity. Archetypes also allow us to store contiguous and
//! ordered arrays of each component. For example, all `Position` components
//! for all entities in an archetype are stored together in one array, and
//! can be returned from queries as a slice. All component arrays for an
//! archetype are stored in the same order and are necessarily the same
//! length, allowing us to index them together to access the components for
//! a single entity.
//!
//! Because these components are stored contiguously in memory, iterating
//! through the components in an archetype is extremely performant as
//! it offers perfect cache locality. By storing each component type in
//! its own array, we only need to access the memory containing components
//! actually reqested by the query's view (see the
//! [query module](../query/index.html)).
//!
//! One of the disadvantages of archetypes is that there are discontinuities
//! between component arrays of different archetypes. In practise this causes
//! approximately one additional L2/3 cache miss per unique entity layout that
//! exists among the result set of a query.
//!
//! Legion mitigates this by conservatively packing archetype component
//! slices next to each other. A component slice is considered eligible
//! for packing if its components have remained stable for some time (i.e no
//! entities have been added or removed from the archetype recently) and
//! an estimate of potential saved cache misses passes a "worthwhile"
//! threshold.
//!
//! By default, legion will pack component slices in the order in which
//! the archetypes were created. This matches the order in which queries will
//! attempt to access each slice. However, most queries do not access all
//! archetypes that contain a certain component - more likely they will skip
//! past many archetypes due to other filters (such as only being interested
//! in archetypes which also contain another specific component).
//!
//! We can provide hints to a world about how it should pack archetypes by
//! declaring groups with the world's [options](../world/struct.WorldOptions.html)
//! which can be provided while constructing the world. Component groups can be
//! used to accelerate the largest and most common queries by optmizing the data
//! layout for those queries.
//!
//! Each component type in a world may belong to precisely one group. A group is
//! a set of components which are frequently queried for together. Queries which
//! match a group will not suffer from performance loss due to archetype
//! fragmentation.
//!
//! Each group implicitly also defines sub-groups, such that the group
//! `(A, B, C, D)` also defines the groups `(A, B, C)` and `(A, B)`.
//!
//! Groups are defined before constructing a world and are passed in the world's
//! options.
//!
//! ```
//! # use legion::*;
//! // our component types
//! struct A;
//! struct B;
//! struct C;
//!
//! // create a world optimized for cases where (A, B) and/or
//! // (A, B, C) are significant queries.
//! let group = <(A, B, C)>::to_group();
//! let options = WorldOptions { groups: vec![group] };
//! let world = World::new(options);
//! ```

pub use crate::internals::{
    cons::{ConsAppend, ConsFlatten},
    hash::{ComponentTypeIdHasher, U64Hasher},
    insert::{
        ArchetypeSource, ArchetypeWriter, ComponentSource, ComponentWriter, IntoComponentSource,
        IntoSoa, UnknownComponentWriter,
    },
    storage::{
        archetype::{Archetype, ArchetypeIndex, EntityLayout},
        component::{Component, ComponentTypeId},
        group::{Group, GroupDef, GroupSource},
        index::SearchIndex,
        packed::PackedStorage,
        ComponentIndex, ComponentMeta, ComponentSlice, ComponentSliceMut, ComponentStorage,
        Components, Epoch, MultiMut, PackOptions, UnknownComponentStorage, Version,
    },
};
