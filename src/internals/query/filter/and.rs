use super::{
    not::Not, or::Or, passthrough::Passthrough, ActiveFilter, DynamicFilter, FilterResult,
    GroupMatcher, LayoutFilter,
};
use crate::internals::{query::view::Fetch, storage::ComponentTypeId, world::WorldId};

/// A filter which requires all filters within `T` match.
#[derive(Debug, Clone)]
pub struct And<T> {
    pub(super) filters: T,
}

macro_rules! and_filter {
    ($head_ty:ident) => {
        impl_and_filter!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_and_filter!($head_ty, $( $tail_ty ),*);
        and_filter!($( $tail_ty ),*);
    );
}

macro_rules! impl_and_filter {
    ( $( $ty:ident ),* ) => {
        impl<$( $ty ),*> ActiveFilter for And<($( $ty, )*)> {}

        impl<$( $ty: Default ),*> Default for And<($( $ty, )*)> {
            fn default() -> Self {
                Self {
                    filters: ($( $ty::default(), )*)
                }
            }
        }

        impl<$( $ty: GroupMatcher ),*> GroupMatcher for And<($( $ty, )*)> {
            fn can_match_group() -> bool {
                $( $ty::can_match_group() )&&*
            }
            fn group_components() -> Vec<ComponentTypeId> {
                let mut components = Vec::new();
                $( components.extend($ty::group_components()); )*
                components
            }
        }

        impl<$( $ty: LayoutFilter ),*> LayoutFilter for And<($( $ty, )*)> {
            #[inline]
            fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = &self.filters;
                let mut result = FilterResult::Defer;
                $( result = result.coalesce_and($ty.matches_layout(components)); )*
                result
            }
        }

        impl<$( $ty: DynamicFilter ),*> DynamicFilter for And<($( $ty, )*)> {
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
                $( result = result.coalesce_and($ty.matches_archetype(fetch)); )*
                result
            }
        }

        impl<$( $ty ),*> std::ops::Not for And<($( $ty, )*)> {
            type Output = Not<Self>;

            #[inline]
            fn not(self) -> Self::Output {
                Not { filter: self }
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitAnd<Rhs> for And<($( $ty, )*)> {
            type Output = And<($( $ty, )* Rhs)>;

            #[inline]
            fn bitand(self, rhs: Rhs) -> Self::Output {
                #![allow(non_snake_case)]
                let ($( $ty, )*) = self.filters;
                And {
                    filters: ($( $ty, )* rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitAnd<Passthrough> for And<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitand(self, _: Passthrough) -> Self::Output {
                self
            }
        }

        impl<$( $ty ),*, Rhs: ActiveFilter> std::ops::BitOr<Rhs> for And<($( $ty, )*)> {
            type Output = Or<(Self, Rhs)>;

            #[inline]
            fn bitor(self, rhs: Rhs) -> Self::Output {
                Or {
                    filters: (self, rhs),
                }
            }
        }

        impl<$( $ty ),*> std::ops::BitOr<Passthrough> for And<($( $ty, )*)> {
            type Output = Self;

            #[inline]
            fn bitor(self, _: Passthrough) -> Self::Output {
                self
            }
        }
    };
}

#[cfg(feature = "extended-tuple-impls")]
and_filter!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
and_filter!(A, B, C, D, E, F, G, H);
