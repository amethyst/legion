use crate::{
    query::{filter::EntityFilter, view::View, Query},
    storage::{archetype::ArchetypeIndex, component::ComponentTypeId},
    subworld::Access,
    world::World,
};
use bit_set::BitSet;

/// Structure describing the resource and component access conditions of the system.
#[derive(Debug, Clone)]
pub struct SystemAccess {
    //pub resources: Access<ResourceTypeId>,
    pub components: Access<ComponentTypeId>,
}

/// Provides an abstraction across tuples of queries for system closures.
pub trait QuerySet: Send + Sync {
    /// Evaluates the queries and records which archetypes they require access to into a bitset.
    fn filter_archetypes(&mut self, world: &World, archetypes: &mut BitSet);
}

macro_rules! queryset_tuple {
    ($head_ty:ident) => {
        impl_queryset_tuple!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_queryset_tuple!($head_ty, $( $tail_ty ),*);
        queryset_tuple!($( $tail_ty ),*);
    );
}

macro_rules! impl_queryset_tuple {
    ($($ty: ident),*) => {
            #[allow(unused_parens, non_snake_case)]
            impl<$( $ty, )*> QuerySet for ($( $ty, )*)
            where
                $( $ty: QuerySet, )*
            {
                fn filter_archetypes(&mut self, world: &World, bitset: &mut BitSet) {
                    let ($($ty,)*) = self;

                    $( $ty.filter_archetypes(world, bitset); )*
                }
            }
    };
}

#[cfg(feature = "extended-tuple-impls")]
queryset_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
queryset_tuple!(A, B, C, D, E, F, G, H);

impl QuerySet for () {
    fn filter_archetypes(&mut self, _: &World, _: &mut BitSet) {}
}

impl<AV, AF> QuerySet for Query<AV, AF>
where
    AV: for<'v> View<'v> + Send + Sync,
    AF: EntityFilter,
{
    fn filter_archetypes(&mut self, world: &World, bitset: &mut BitSet) {
        for &ArchetypeIndex(arch) in self.find_archetypes(world) {
            bitset.insert(arch as usize);
        }
    }
}
