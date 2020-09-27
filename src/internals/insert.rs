//! Contains types related to inserting new entities into a [World](../world/struct.World.html)

use super::entity::Entity;
use super::query::filter::{FilterResult, LayoutFilter};
use super::storage::{
    archetype::{Archetype, ArchetypeIndex, EntityLayout},
    component::{Component, ComponentTypeId},
    ComponentIndex, ComponentStorage, MultiMut, UnknownComponentStorage,
};
use std::marker::PhantomData;

/// Provides access to writers for writing new entities into an archetype in a world.
///
/// Users must claim all components contained in the archetype and write an equal number
/// of components to each as the number of entities pushed to the writer.
pub struct ArchetypeWriter<'a> {
    arch_index: ArchetypeIndex,
    archetype: &'a mut Archetype,
    components: MultiMut<'a>,
    claimed: u128,
    initial_count: usize,
}

impl<'a> ArchetypeWriter<'a> {
    /// Constructs a new archetype writer.
    pub fn new(
        arch_index: ArchetypeIndex,
        archetype: &'a mut Archetype,
        components: MultiMut<'a>,
    ) -> Self {
        let initial_count = archetype.entities().len();
        Self {
            arch_index,
            archetype,
            components,
            claimed: 0,
            initial_count,
        }
    }

    /// Returns the archetype being written to.
    pub fn archetype(&self) -> &Archetype {
        &self.archetype
    }

    fn mark_claimed(&mut self, type_id: ComponentTypeId) {
        let component_index = self
            .archetype
            .layout()
            .component_types()
            .iter()
            .position(|t| t == &type_id)
            .expect("archetype does not contain component");
        let mask = 1u128 << component_index;
        assert!(self.claimed & mask == 0, "component type already claimed");
        self.claimed |= mask;
    }

    /// Claims a component storage for a given component.
    ///
    /// # Panics
    /// Panics if the storage for the requested component type has already been claimed
    /// or does not exist in the archetype.
    pub fn claim_components<T: Component>(&mut self) -> ComponentWriter<'a, T> {
        let type_id = ComponentTypeId::of::<T>();
        self.mark_claimed(type_id);

        ComponentWriter {
            components: unsafe { self.components.claim::<T>() }.unwrap(),
            archetype: self.arch_index,
        }
    }

    /// Claims a component storage for a given component.
    ///
    /// # Panics
    /// Panics if the storage for the requested component type has already been claimed
    /// or does not exist in the archetype.
    pub fn claim_components_unknown(
        &mut self,
        type_id: ComponentTypeId,
    ) -> UnknownComponentWriter<'a> {
        self.mark_claimed(type_id);

        UnknownComponentWriter {
            components: unsafe { self.components.claim_unknown(type_id) }.unwrap(),
            archetype: self.arch_index,
        }
    }

    /// Pushes an entity ID into the archetype.
    pub fn push(&mut self, entity: Entity) {
        self.archetype.push(entity);
    }

    /// Reserves capacity for at least `additional` extra entity IDs in the archetype.
    pub fn reserve(&mut self, additional: usize) {
        self.archetype.reserve(additional)
    }

    /// Returns a slice of entities inserted by this writer, and the component index of the first inserted entity.
    pub fn inserted(&self) -> (ComponentIndex, &[Entity]) {
        let start = self.initial_count;
        (ComponentIndex(start), &self.archetype.entities()[start..])
    }
}

impl<'a> Drop for ArchetypeWriter<'a> {
    fn drop(&mut self) {
        assert_eq!(
            self.claimed.count_ones() as usize,
            self.archetype.layout().component_types().len()
        );
    }
}

/// Provides the ability to append new components to the entities in an archetype.
pub struct ComponentWriter<'a, T: Component> {
    components: &'a mut T::Storage,
    archetype: ArchetypeIndex,
}

impl<'a, T: Component> ComponentWriter<'a, T> {
    /// Writes the given components into the component storage.
    ///
    /// # Safety
    /// `ptr` must point to a valid array of `T` of length at least as long as `len`.
    /// The data in this array will be memcopied into the world's internal storage.
    /// If the component type is not `Copy`, then the caller must ensure that the memory
    /// copied is not accessed until it is re-initialized. It is recommended to immediately
    /// `std::mem::forget` the source after calling `extend_memcopy`.
    pub unsafe fn extend_memcopy(&mut self, ptr: *const T, len: usize) {
        self.components.extend_memcopy(self.archetype, ptr, len)
    }

    /// Ensures that the given spare capacity is available in the target storage location.
    /// Calling this function before calling `extend_memcopy` is not required, but may
    /// avoid additional vector resizes.
    pub fn ensure_capacity(&mut self, space: usize) {
        self.components.ensure_capacity(self.archetype, space);
    }
}

/// Provides the ability to append new components to the entities in an archetype.
pub struct UnknownComponentWriter<'a> {
    components: &'a mut dyn UnknownComponentStorage,
    archetype: ArchetypeIndex,
}

impl<'a> UnknownComponentWriter<'a> {
    /// Writes the given components into the component storage.
    ///
    /// # Safety
    /// `ptr` must point to a valid array of the correct component type of length at least as
    /// long as `len`.
    /// The data in this array will be memcopied into the world's internal storage.
    /// If the component type is not `Copy`, then the caller must ensure that the memory
    /// copied is not accessed until it is re-initialized. It is recommended to immediately
    /// `std::mem::forget` the source after calling `extend_memcopy`.
    pub unsafe fn extend_memcopy_raw(&mut self, ptr: *const u8, len: usize) {
        self.components.extend_memcopy_raw(self.archetype, ptr, len)
    }

    /// Ensures that the given spare capacity is available in the target storage location.
    /// Calling this function before calling `extend_memcopy` is not required, but may
    /// avoid additional vector resizes.
    pub fn ensure_capacity(&mut self, space: usize) {
        self.components.ensure_capacity(self.archetype, space);
    }

    /// Moves all of the components from the given storage location into this writer's storage.
    pub fn move_archetype_from(
        &mut self,
        src_archetype: ArchetypeIndex,
        src: &mut dyn UnknownComponentStorage,
    ) {
        src.transfer_archetype(src_archetype, self.archetype, self.components);
    }

    /// Copies all of the components from the given storage location into this writer's storage.
    pub fn copy_archetype_from(
        &mut self,
        src_archetype: ArchetypeIndex,
        src: &dyn UnknownComponentStorage,
    ) {
        src.copy_archetype(src_archetype, self.archetype, self.components);
    }

    /// Moves a single component from the given storage location into this writer's storage.
    pub fn move_component_from(
        &mut self,
        src_archetype: ArchetypeIndex,
        src_component: ComponentIndex,
        src: &mut dyn UnknownComponentStorage,
    ) {
        src.transfer_component(
            src_archetype,
            src_component,
            self.archetype,
            self.components,
        );
    }
}

/// Defines a type which can describe the layout of an archetype.
pub trait ArchetypeSource {
    /// A filter which finds existing archetypes which match the layout.
    type Filter: LayoutFilter;

    /// Returns the archetype source's filter.
    fn filter(&self) -> Self::Filter;

    /// Constructs a new entity layout.
    fn layout(&mut self) -> EntityLayout;
}

/// Describes a type which can write entity components into a world.
pub trait ComponentSource: ArchetypeSource {
    /// Writes components for new entities into an archetype.
    fn push_components<'a>(
        &mut self,
        writer: &mut ArchetypeWriter<'a>,
        entities: impl Iterator<Item = Entity>,
    );
}

/// A collection with a known length.
pub trait KnownLength {
    fn len(&self) -> usize;
}

/// Converts a type into a [ComponentSource](trait.ComponentSource.html).
pub trait IntoComponentSource {
    /// The output component source.
    type Source: ComponentSource;

    /// Converts this structure into a component source.
    fn into(self) -> Self::Source;
}

/// A wrapper for a Structure of Arrays used for efficient entity insertion.
pub struct Soa<T> {
    vecs: T,
}

/// A single vector component inside an SoA.
pub struct SoaElement<T> {
    _phantom: PhantomData<T>,
    ptr: *mut T,
    len: usize,
    capacity: usize,
}

unsafe impl<T> Send for SoaElement<T> {}
unsafe impl<T> Sync for SoaElement<T> {}

impl<T> Drop for SoaElement<T> {
    fn drop(&mut self) {
        unsafe {
            // reconstruct the original vector, but with length set to the remaining elements
            let _ = Vec::from_raw_parts(self.ptr, self.len, self.capacity);
        }
    }
}

/// Desribes a type which can convert itself into an [SoA](trait.Soa.html)
/// representation for entity intertion.
pub trait IntoSoa {
    /// The output entity source.
    type Source;

    /// Converts this into an SoA component source.
    fn into_soa(self) -> Self::Source;
}

/// A wrapper for an Array of Structures used for entity intertions.
pub struct Aos<T, Iter> {
    _phantom: PhantomData<T>,
    iter: Iter,
}

impl<T, Iter> Aos<T, Iter>
where
    Iter: Iterator<Item = T>,
{
    /// Constructs a new Aos.
    fn new(iter: Iter) -> Self {
        Self {
            iter,
            _phantom: PhantomData,
        }
    }
}

impl<I> IntoComponentSource for I
where
    I: IntoIterator,
    Aos<I::Item, I::IntoIter>: ComponentSource,
{
    type Source = Aos<I::Item, I::IntoIter>;

    fn into(self) -> Self::Source {
        <Self::Source>::new(self.into_iter())
    }
}

/// A layout filter used to select the appropriate archetype for interting
/// entities from a component source into a world.
pub struct ComponentSourceFilter<T>(PhantomData<T>);

impl<T> Default for ComponentSourceFilter<T> {
    fn default() -> Self {
        ComponentSourceFilter(PhantomData)
    }
}

impl LayoutFilter for ComponentSourceFilter<()> {
    fn matches_layout(&self, components: &[ComponentTypeId]) -> FilterResult {
        FilterResult::Match(components.is_empty())
    }
}

impl<Iter> IntoComponentSource for Aos<(), Iter>
where
    Iter: Iterator,
    Aos<(), Iter>: ComponentSource,
{
    type Source = Self;
    fn into(self) -> Self::Source {
        self
    }
}

impl<Iter> ArchetypeSource for Aos<(), Iter>
where
    Iter: Iterator,
{
    type Filter = ComponentSourceFilter<()>;

    fn filter(&self) -> Self::Filter {
        ComponentSourceFilter(PhantomData)
    }

    fn layout(&mut self) -> EntityLayout {
        EntityLayout::default()
    }
}

impl<Iter> ComponentSource for Aos<(), Iter>
where
    Iter: Iterator,
{
    fn push_components<'a>(
        &mut self,
        writer: &mut ArchetypeWriter<'a>,
        mut entities: impl Iterator<Item = Entity>,
    ) {
        for _ in &mut self.iter {
            let entity = entities.next().unwrap();
            writer.push(entity);
        }
    }
}

impl<Iter> KnownLength for Aos<(), Iter>
where
    Iter: ExactSizeIterator,
{
    fn len(&self) -> usize {
        self.iter.len()
    }
}

macro_rules! component_source {
    ($head_ty:ident) => {
        impl_component_source!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_component_source!($head_ty, $( $tail_ty ),*);
        component_source!($( $tail_ty ),*);
    );
}

macro_rules! impl_component_source {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Component ),*> LayoutFilter for ComponentSourceFilter<($( $ty, )*)> {
            fn matches_layout(
                &self,
                components: &[ComponentTypeId],
            ) -> FilterResult {
                let types = &[$( ComponentTypeId::of::<$ty>() ),*];
                FilterResult::Match(components.len() == types.len() && types.iter().all(|t| components.contains(t)))
            }
        }

        paste::item! {
            impl<$( $ty: Component ),*> Soa<($( SoaElement<$ty>, )*)> {
                fn validate_equal_length(vecs: &($( Vec<$ty>, )*)) -> bool {
                    #![allow(non_snake_case)]

                    let len = vecs.0.len();
                    let ($( [<$ty _vec>], )*) = vecs;
                    $(
                        if [<$ty _vec>].len() != len {
                            return false;
                        }
                    )*

                    true
                }
            }

            impl<$( $ty: Component ),*> IntoSoa for ($( Vec<$ty>, )*) {
                type Source = Soa<($( SoaElement<$ty>, )*)>;

                fn into_soa(self) -> Self::Source {
                    #![allow(non_snake_case)]

                    if !<Self::Source>::validate_equal_length(&self) {
                        panic!("all component vecs must have equal length");
                    }

                    let ($([<$ty _vec>], )*) = self;
                    Soa {
                        vecs: ($({
                            let mut [<$ty _vec>] = std::mem::ManuallyDrop::new([<$ty _vec>]);
                            SoaElement {
                                _phantom: PhantomData,
                                capacity: [<$ty _vec>].capacity(),
                                len: [<$ty _vec>].len(),
                                ptr: [<$ty _vec>].as_mut_ptr(),
                            }
                        }, )*),
                    }
                }
            }
        }

        // impl<$( $ty: Component ),*> IntoComponentSource for ($( Vec<$ty>, )*) {
        //     type Source = Soa<($( SoaElement<$ty>, )*)>;
        //     fn into(self) -> Self::Source { Soa::<($( SoaElement<$ty>, )*)>::new(self) }
        // }

        impl<$( $ty ),*> IntoComponentSource for Soa<($( SoaElement<$ty>, )*)>
        where
            Soa<($( SoaElement<$ty>, )*)>: ComponentSource
        {
            type Source = Self;
            fn into(self) -> Self::Source { self }
        }

        impl<$( $ty: Component ),*> ArchetypeSource for Soa<($( SoaElement<$ty>, )*)> {
            type Filter = ComponentSourceFilter<($( $ty, )*)>;

            fn filter(&self) -> Self::Filter {
                ComponentSourceFilter(PhantomData)
            }

            fn layout(&mut self) -> EntityLayout {
                let mut layout = EntityLayout::default();
                $(
                    layout.register_component::<$ty>();
                )*
                layout
            }
        }

        impl<$( $ty: Component ),*> ComponentSource for Soa<($( SoaElement<$ty>, )*)> {
            paste::item! {
                fn push_components<'a>(
                    &mut self,
                    writer: &mut ArchetypeWriter<'a>,
                    mut entities: impl Iterator<Item = Entity>,
                ) {
                    #![allow(unused_variables)]
                    #![allow(non_snake_case)]

                    let len = self.vecs.0.len;
                    for _ in 0..len {
                        writer.push(entities.next().unwrap());
                    }

                    let ($( [<$ty _vec>], )*) = &mut self.vecs;

                    $(
                        let mut target = writer.claim_components::<$ty>();
                        unsafe {
                            target.extend_memcopy([<$ty _vec>].ptr, len);
                            [<$ty _vec>].len = 0
                        }
                    )*
                }
            }
        }

        impl<$( $ty: Component ),*> KnownLength for Soa<($( SoaElement<$ty>, )*)> {
            fn len(&self) -> usize {
                self.vecs.0.len
            }
        }

        impl<Iter, $( $ty: Component ),*> IntoComponentSource for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>,
            Aos<($( $ty, )*), Iter>: ComponentSource
        {
            type Source = Self;
            fn into(self) -> Self::Source { self }
        }

        // impl<Iter, $( $ty: Component ),*> LayoutFilter for Aos<($( $ty, )*), Iter>
        // where
        //     Iter: Iterator<Item = ($( $ty, )*)>
        // {
        //     fn matches_layout(
        //         &self,
        //         components: &[ComponentTypeId],
        //     ) -> Option<bool> {
        //         let types = &[$( ComponentTypeId::of::<$ty>() ),*];
        //         Some(components.len() == types.len() && types.iter().all(|t| components.contains(t)))
        //     }
        // }

        impl<Iter, $( $ty: Component ),*> ArchetypeSource for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>
        {
            type Filter = ComponentSourceFilter<($( $ty, )*)>;

            fn filter(&self) -> Self::Filter {
                ComponentSourceFilter(PhantomData)
            }

            fn layout(&mut self) -> EntityLayout {
                let mut layout = EntityLayout::default();
                $(
                    layout.register_component::<$ty>();
                )*
                layout
            }
        }

        impl<Iter, $( $ty: Component ),*> ComponentSource for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)>
        {
            paste::item! {
                fn push_components<'a>(
                    &mut self,
                    writer: &mut ArchetypeWriter<'a>,
                    mut entities: impl Iterator<Item = Entity>,
                ) {
                    #![allow(non_snake_case)]

                    $(
                        let mut [<$ty _target>] = writer.claim_components::<$ty>();
                    )*

                    let (min_size, _) = self.iter.size_hint();
                    $( [<$ty _target>].ensure_capacity(min_size); )*

                    let mut count = 0;
                    for ($( $ty, )*) in &mut self.iter {
                        count += 1;

                        $(
                            unsafe {
                                [<$ty _target>].extend_memcopy(&$ty, 1);
                                std::mem::forget($ty);
                            }
                        )*
                    }

                    for _ in 0..count {
                        let entity = entities.next().unwrap();
                        writer.push(entity);
                    }
                }
            }
        }

        impl<Iter, $( $ty: Component ),*> KnownLength for Aos<($( $ty, )*), Iter>
        where
            Iter: Iterator<Item = ($( $ty, )*)> + ExactSizeIterator
        {
            fn len(&self) -> usize {
                self.iter.len()
            }
        }
    };
}

#[cfg(feature = "extended-tuple-impls")]
component_source!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(not(feature = "extended-tuple-impls"))]
component_source!(A, B, C, D, E, F, G, H);
