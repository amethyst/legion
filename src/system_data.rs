//! TODO

use crate::internals::systems::resources::{ResourceTypeId, UnsafeResources};

/// TODO
pub trait SystemResources<'a> {
    /// TODO
    type ConsConcatArg;

    /// TODO
    fn resources() -> Self::ConsConcatArg;

    /// TODO
    unsafe fn fetch_unchecked(resources: &'a UnsafeResources) -> Self;

    /// TODO
    fn read_resource_type_ids() -> Vec<ResourceTypeId>;

    /// TODO
    fn write_resource_type_ids() -> Vec<ResourceTypeId>;
}
