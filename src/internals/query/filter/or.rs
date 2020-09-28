use super::{
    and::And, not::Not, passthrough::Passthrough, ActiveFilter, DynamicFilter, FilterResult,
    GroupMatcher, LayoutFilter,
};
use crate::internals::{query::view::Fetch, storage::component::ComponentTypeId, world::WorldId};

/// A filter which requires any filter within `T` match.
#[derive(Debug, Clone)]
pub struct Or<T> {
    pub(super) filters: T,
}

macro_rules! or_filter {
    ($head_ty:ident) => {
        impl_or_filter!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_or_filter!($head_ty, $( $tail_ty ),*);
        or_filter!($( $tail_ty ),*);
    );
}

macro_rules! impl_or_filter {
    ( $( $ty:ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for Or<($( $ty, )*)> {}

        impl<$( $ty: Default ),*> Default for Or<($( $ty, )*)> {
            fn default() -> Self {
                Self {
                    filters: ($( $ty::default(), )*)
                }
            }
        }

        impl<$( $ty ),*> GroupMatcher for Or<($( $ty, )*)> {
            fn can_match_group() -> bool {
                false
            }
            fn group_components() -> Vec<ComponentTypeId> {
                vec![]
            }
        }

        impl<$( $ty: LayoutFilter ),*> LayoutFilter for Or<($( $ty, )*)> {
            #[inline]
            fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result = FilterResult::Defer;
                $( result = result.coalesce_or($ty.matches_layout(components)); )*
                result
            }
        }

        impl<$( $ty: DynamicFilter ),*> DynamicFilter for Or<($( $ty, )*)> {
            #[inline]
            fn prepare(&mut self, world: WorldId) {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                $( $ty.prepare(world); )*
            }

            #[inline]
            fn matches_archetype<Fet: Fetch>(&mut self, fetch: &Fet) -> FilterResult {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &mut self.filters;
                let mut result = FilterResult::Defer;
                $( result = result.coalesce_or($ty.matches_archetype(fetch)); )*
                result
            }
        }

        impl<$( $ty ),*> std::ops::Not for Or<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for Or<($( $ty, )*)> {
            type Output = And<(Self, Rhs)>;

            #[inline]
            fn bitand(self, rhs: Rhs) -> Self::Output {
                And {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitAnd<Passthrough> for Or<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitand(self, _: Passthrough) -> Self::Output {
                self
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for Or<($( $ty, )*)> {
            type Output =  Or<($( $ty, )* Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                Or {
                    filters: ($( $ty, )* rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitOr<Passthrough> for Or<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitor(self, _: Passthrough) -> Self::Output {
                self
            }
        }
    };
}

#[cfg(feature = "extended-tuple-impls")]
or_filter!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
or_filter!(A, B, C, D, E, F, G, H);
