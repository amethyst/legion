use crate::experimental::borrow::ComponentRef;
use crate::experimental::borrow::ComponentRefMut;
use crate::experimental::borrow::Ref;
use crate::experimental::entity::BlockAllocator;
use crate::experimental::entity::Entity;
use crate::experimental::entity::EntityAllocator;
use crate::experimental::entity::EntityLocation;
use crate::experimental::filter::ArchetypeFilterData;
use crate::experimental::filter::ChunkFilterData;
use crate::experimental::filter::Filter;
use crate::experimental::filter::FilterResult;
use crate::experimental::storage::ArchetypeDescription;
use crate::experimental::storage::Component;
use crate::experimental::storage::ComponentStorage;
use crate::experimental::storage::ComponentTypeId;
use crate::experimental::storage::Storage;
use crate::experimental::storage::Tag;
use crate::experimental::storage::TagStorage;
use crate::experimental::storage::TagTypeId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::iter::Peekable;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub struct Universe {
    allocator: Arc<Mutex<BlockAllocator>>,
}

impl Universe {
    pub fn new() -> Self { Self::default() }
    pub fn create_world(&self) -> World { World::new(EntityAllocator::new(self.allocator.clone())) }
}

impl Default for Universe {
    fn default() -> Self {
        Self {
            allocator: Arc::new(Mutex::new(BlockAllocator::new())),
        }
    }
}

pub struct World {
    pub(crate) archetypes: Storage,
    entity_allocator: EntityAllocator,
}

impl World {
    pub fn new(allocator: EntityAllocator) -> Self {
        Self {
            archetypes: Storage::default(),
            entity_allocator: allocator,
        }
    }

    pub fn insert<T, C>(&mut self, mut tags: T, components: C) -> &[Entity]
    where
        T: TagSet,
        C: IntoComponentSource,
    {
        let mut components = components.into();
        let archetype_id = self.find_or_create_archetype(&mut tags, &mut components);

        self.entity_allocator.clear_allocation_buffer();

        while !components.is_empty() {
            let chunk_id = self.find_or_create_chunk(archetype_id, &mut tags);
            let mut archetype = unsafe {
                self.archetypes
                    .data_unchecked(archetype_id)
                    .deref()
                    .get_mut()
            };
            let component_storage = archetype.component_chunk_mut(chunk_id).unwrap();

            let allocated = components.write(&mut self.entity_allocator, component_storage);
            let start = component_storage.len() - allocated;
            let added = component_storage.entities().iter().enumerate().skip(start);
            for (i, e) in added {
                let location = EntityLocation::new(archetype_id, chunk_id, i);
                self.entity_allocator.set_location(e.index(), location);
            }
        }

        self.entity_allocator.allocation_buffer()
    }

    pub fn delete(&mut self, entity: Entity) -> bool {
        let deleted = self.entity_allocator.delete_entity(entity);

        if deleted {
            let location = self.entity_allocator.get_location(entity.index()).unwrap();
            let mut archetype = self
                .archetypes
                .data(location.archetype())
                .unwrap()
                .get_mut();
            let chunk = archetype.component_chunk_mut(location.chunk()).unwrap();
            if let Some(swapped) = chunk.swap_remove(location.component()) {
                self.entity_allocator
                    .set_location(swapped.index(), location);
            }
        }

        deleted
    }

    pub fn is_alive(&self, entity: Entity) -> bool { self.entity_allocator.is_alive(entity) }

    pub fn get_component<T: Component>(&mut self, entity: Entity) -> Option<ComponentRef<T>> {
        if !self.is_alive(entity) {
            return None;
        }

        let location = self.entity_allocator.get_location(entity.index())?;
        let (archetype_borrow, archetype) = unsafe {
            self.archetypes
                .data(location.archetype())?
                .get()
                .deconstruct()
        };
        let chunk = archetype.component_chunk(location.chunk())?;
        let (slice_borrow, slice) = unsafe {
            chunk
                .components(ComponentTypeId::of::<T>())?
                .data_slice::<T>()
                .deconstruct()
        };
        let component = slice.get(location.component())?;

        Some(ComponentRef::new(archetype_borrow, slice_borrow, component))
    }

    pub fn get_component_mut<T: Component>(
        &mut self,
        entity: Entity,
    ) -> Option<ComponentRefMut<T>> {
        if !self.is_alive(entity) {
            return None;
        }

        let location = self.entity_allocator.get_location(entity.index())?;
        let (archetype_borrow, archetype) = unsafe {
            self.archetypes
                .data(location.archetype())?
                .get()
                .deconstruct()
        };
        let chunk = archetype.component_chunk(location.chunk())?;
        let (slice_borrow, slice) = unsafe {
            chunk
                .components(ComponentTypeId::of::<T>())?
                .data_slice_mut::<T>()
                .deconstruct()
        };
        let component = slice.get_mut(location.component())?;

        Some(ComponentRefMut::new(
            archetype_borrow,
            slice_borrow,
            component,
        ))
    }

    pub fn get_tag<T: Tag>(&self, entity: Entity) -> Option<Ref<T>> {
        if !self.is_alive(entity) {
            return None;
        }

        let location = self.entity_allocator.get_location(entity.index())?;
        let (archetype_borrow, archetype) = unsafe {
            self.archetypes
                .data(location.archetype())?
                .get()
                .deconstruct()
        };
        let tags = archetype.tags(TagTypeId::of::<T>())?;
        let tag = unsafe { tags.data_slice::<T>().get(location.chunk())? };

        Some(Ref::new(archetype_borrow, tag))
    }

    fn find_or_create_archetype<T, C>(&mut self, tags: &mut T, components: &mut C) -> usize
    where
        T: TagLayout,
        C: ComponentLayout,
    {
        {
            let archetype_data = ArchetypeFilterData {
                component_types: self.archetypes.component_types(),
                tag_types: self.archetypes.tag_types(),
            };

            let tag_iter = tags.collect(archetype_data);
            let tag_matches =
                tag_iter.map(|x| <T as Filter<'_, ArchetypeFilterData<'_>>>::is_match(tags, &x));
            let component_iter = components.collect(archetype_data);
            let component_matches = component_iter.map(|x| components.is_match(&x));
            if let Some(i) = tag_matches
                .zip(component_matches)
                .enumerate()
                .filter(|(_, (t, c))| t.is_pass() && c.is_pass())
                .map(|(i, _)| i)
                .next()
            {
                return i;
            };
        }

        let mut description = ArchetypeDescription::default();
        tags.tailor_archetype(&mut description);
        components.tailor_archetype(&mut description);

        let (index, _) = self.archetypes.alloc_archetype(&description);
        index
    }

    fn find_or_create_chunk<T>(&mut self, archetype: usize, tags: &mut T) -> usize
    where
        T: TagSet,
    {
        let mut archetype_data =
            unsafe { self.archetypes.data_unchecked(archetype).deref().get_mut() };

        {
            let chunk_filter_data = ChunkFilterData {
                archetype_data: archetype_data.deref(),
            };

            let chunk_iter = tags.collect(chunk_filter_data);
            let index = chunk_iter
                .map(|x| <T as Filter<'_, ChunkFilterData<'_>>>::is_match(tags, &x))
                .zip(archetype_data.iter_component_chunks())
                .enumerate()
                .filter(|(_, (matches, components))| matches.is_pass() && !components.is_full())
                .map(|(i, _)| i)
                .next();

            if let Some(i) = index {
                return i;
            }
        }

        let (id, chunk_tags, _) = archetype_data.alloc_chunk();
        tags.write_tags(chunk_tags);
        id
    }
}

pub trait ComponentLayout: Sized + for<'a> Filter<'a, ArchetypeFilterData<'a>> {
    fn tailor_archetype(&self, archetype: &mut ArchetypeDescription);
}

pub trait TagLayout: Sized + for<'a> Filter<'a, ArchetypeFilterData<'a>> {
    fn tailor_archetype(&self, archetype: &mut ArchetypeDescription);
}

pub trait TagSet: TagLayout + for<'a> Filter<'a, ChunkFilterData<'a>> {
    fn write_tags(&self, tags: &mut HashMap<TagTypeId, TagStorage>);
}

pub trait ComponentSource: ComponentLayout {
    fn is_empty(&mut self) -> bool;
    fn write(&mut self, allocator: &mut EntityAllocator, chunk: &mut ComponentStorage) -> usize;
}

pub trait IntoComponentSource {
    type Source: ComponentSource;

    fn into(self) -> Self::Source;
}

pub struct ComponentTupleSet<T, I>
where
    I: Iterator<Item = T>,
{
    iter: Peekable<I>,
}

impl<T, I> From<I> for ComponentTupleSet<T, I>
where
    I: Iterator<Item = T>,
    ComponentTupleSet<T, I>: ComponentSource,
{
    fn from(iter: I) -> Self {
        ComponentTupleSet {
            iter: iter.peekable(),
        }
    }
}

impl<I> IntoComponentSource for I
where
    I: IntoIterator,
    ComponentTupleSet<I::Item, I::IntoIter>: ComponentSource,
{
    type Source = ComponentTupleSet<I::Item, I::IntoIter>;

    fn into(self) -> Self::Source {
        ComponentTupleSet {
            iter: self.into_iter().peekable(),
        }
    }
}

mod tuple_impls {
    use super::*;
    use crate::experimental::storage::Component;
    use crate::experimental::storage::ComponentTypeId;
    use crate::experimental::storage::SliceVecIter;
    use crate::experimental::storage::Tag;
    use itertools::Zip;
    use std::iter::Repeat;
    use std::iter::Take;
    use std::slice::Iter;

    macro_rules! impl_data_tuple {
        ( $( $ty: ident => $id: ident ),* ) => {
            impl_data_tuple!(@TAG_SET $( $ty => $id ),*);
            impl_data_tuple!(@COMPONENT_SOURCE $( $ty => $id ),*);
        };
        ( @COMPONENT_SOURCE $( $ty: ident => $id: ident ),* ) => {
            impl<I, $( $ty ),*> ComponentLayout for ComponentTupleSet<($( $ty, )*), I>
            where
                I: Iterator<Item = ($( $ty, )*)>,
                $( $ty: Component ),*
            {
                fn tailor_archetype(&self, archetype: &mut ArchetypeDescription) {
                    #![allow(unused_variables)]
                    $(
                        archetype.register_component::<$ty>();
                    )*
                }
            }

            impl<I, $( $ty ),*> ComponentSource for ComponentTupleSet<($( $ty, )*), I>
            where
                I: Iterator<Item = ($( $ty, )*)>,
                $( $ty: Component ),*
            {
                fn is_empty(&mut self) -> bool {
                    self.iter.peek().is_none()
                }

                fn write(&mut self, allocator: &mut EntityAllocator, chunk: &mut ComponentStorage) -> usize {
                    #![allow(unused_variables)]
                    #![allow(unused_unsafe)]
                    #![allow(non_snake_case)]
                    let space = chunk.capacity() - chunk.len();
                    let (entities, components) = chunk.write();
                    let mut count = 0;

                    unsafe {
                        $(
                            let mut $ty = (&mut *components.get()).get_mut(&ComponentTypeId::of::<$ty>()).unwrap().writer();
                        )*

                        while let Some(($( $id, )*)) = { if count == space { None } else { self.iter.next() } } {
                            let entity = allocator.create_entity();
                            entities.push(entity);
                            $(
                                $ty.push(&[$id]);
                            )*
                            count += 1;
                        }
                    }

                    count
                }
            }

            impl<'a, I, $( $ty ),*> Filter<'a, ArchetypeFilterData<'a>> for ComponentTupleSet<($( $ty, )*), I>
            where
                I: Iterator<Item = ($( $ty, )*)>,
                $( $ty: Component ),*
            {
                type Iter = SliceVecIter<'a, ComponentTypeId>;

                fn collect(&self, source: ArchetypeFilterData<'a>) -> Self::Iter {
                    source.component_types.iter()
                }

                fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                    let types = &[$( ComponentTypeId::of::<$ty>() ),*];
                    Some(types.len() == item.len() && types.iter().all(|t| item.contains(t)))
                }
            }
        };
        ( @TAG_SET $( $ty: ident => $id: ident ),* ) => {
            impl_data_tuple!(@CHUNK_FILTER $( $ty => $id ),*);

            impl<$( $ty ),*> TagSet for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                fn write_tags(&self, tags: &mut HashMap<TagTypeId, TagStorage>) {
                    #![allow(unused_variables)]
                    #![allow(non_snake_case)]
                    let ($($id,)*) = self;
                    $(
                        tags.get_mut(&TagTypeId::of::<$ty>()).unwrap().push($id.clone());
                    )*
                }
            }

            impl <$( $ty ),*> TagLayout for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                fn tailor_archetype(&self, archetype: &mut ArchetypeDescription) {
                    #![allow(unused_variables)]
                    $(
                        archetype.register_tag::<$ty>();
                    )*
                }
            }

            impl<'a, $( $ty ),*> Filter<'a, ArchetypeFilterData<'a>> for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                type Iter = SliceVecIter<'a, TagTypeId>;

                fn collect(&self, source: ArchetypeFilterData<'a>) -> Self::Iter {
                    source.tag_types.iter()
                }

                fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                    let types = &[$( TagTypeId::of::<$ty>() ),*];
                    Some(types.len() == item.len() && types.iter().all(|t| item.contains(t)))
                }
            }
        };
        ( @CHUNK_FILTER $( $ty: ident => $id: ident ),+ ) => {
            impl<'a, $( $ty ),*> Filter<'a, ChunkFilterData<'a>> for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                type Iter = Zip<($( Iter<'a, $ty>, )*)>;

                fn collect(&self, source: ChunkFilterData<'a>) -> Self::Iter {
                    let iters = (
                        $(
                            unsafe {
                                source.archetype_data
                                    .tags(TagTypeId::of::<$ty>())
                                    .unwrap()
                                    .data_slice::<$ty>()
                                    .iter()
                            },
                        )*
                    );

                    itertools::multizip(iters)
                }

                fn is_match(&mut self, item: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                    #![allow(non_snake_case)]
                    let ($( $ty, )*) = self;
                    Some(($( &*$ty, )*) == *item)
                }
            }
        };
        ( @CHUNK_FILTER ) => {
            impl<'a> Filter<'a, ChunkFilterData<'a>> for () {
                type Iter = Take<Repeat<()>>;

                fn collect(&self, source: ChunkFilterData<'a>) -> Self::Iter {
                    std::iter::repeat(()).take(source.archetype_data.len())
                }

                fn is_match(&mut self, _: &<Self::Iter as Iterator>::Item) -> Option<bool> {
                    Some(true)
                }
            }
        };
    }

    impl_data_tuple!();
    impl_data_tuple!(A => a);
    impl_data_tuple!(A => a, B => b);
    impl_data_tuple!(A => a, B => b, C => c);
    impl_data_tuple!(A => a, B => b, C => c, D => d);
    impl_data_tuple!(A => a, B => b, C => c, D => d, E => e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Rot(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Scale(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32, f32, f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Accel(f32, f32, f32);
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    struct Model(u32);
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    struct Static;

    fn create() -> World {
        let universe = Universe::new();
        universe.create_world()
    }

    #[test]
    fn create_universe() { Universe::default(); }

    #[test]
    fn create_world() {
        let universe = Universe::new();
        universe.create_world();
    }

    #[test]
    fn insert() {
        let mut world = create();

        let shared = (1usize, 2f32, 3u16);
        let components = vec![(4f32, 5u64, 6u16), (4f32, 5u64, 6u16)];
        world.insert(shared, components);

        assert_eq!(2, world.entity_allocator.allocation_buffer().len());
    }

    #[test]
    fn get_component() {
        let mut world = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        world.insert(shared, components.clone());

        for (i, e) in world
            .entity_allocator
            .allocation_buffer()
            .to_vec()
            .iter()
            .enumerate()
        {
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
                None => assert_eq!(components.get(i).map(|(x, _)| x), None),
            }
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
                None => assert_eq!(components.get(i).map(|(_, x)| x), None),
            }
        }
    }

    #[test]
    fn get_component_wrong_type() {
        let mut world = create();

        world.insert((), vec![(0f64,)]);

        let entity = *world.entity_allocator.allocation_buffer().get(0).unwrap();

        assert!(world.get_component::<i32>(entity).is_none());
    }

    #[test]
    fn get_tag() {
        let mut world = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        world.insert(shared, components);

        for e in world.entity_allocator.allocation_buffer().iter() {
            assert_eq!(&Static, world.get_tag::<Static>(*e).unwrap().deref());
            assert_eq!(&Model(5), world.get_tag::<Model>(*e).unwrap().deref());
        }
    }

    #[test]
    fn get_tag_wrong_type() {
        let mut world = create();

        world.insert((Static,), vec![(0f64,)]);

        let entity = world.entity_allocator.allocation_buffer().get(0).unwrap();

        assert!(world.get_tag::<Model>(*entity).is_none());
    }

    #[test]
    fn delete() {
        let mut world = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        let entities = world.insert(shared, components).to_vec();

        for e in entities.iter() {
            assert!(world.get_component::<Pos>(*e).is_some());
        }

        for e in entities.iter() {
            world.delete(*e);
            assert!(world.get_component::<Pos>(*e).is_none());
        }
    }

    #[test]
    fn delete_last() {
        let mut world = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        let entities = world.insert(shared, components.clone()).to_vec();

        let last = *entities.last().unwrap();
        world.delete(last);

        for (i, e) in entities.iter().take(entities.len() - 1).enumerate() {
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
                None => assert_eq!(components.get(i).map(|(x, _)| x), None),
            }
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
                None => assert_eq!(components.get(i).map(|(_, x)| x), None),
            }
        }
    }

    #[test]
    fn delete_first() {
        let mut world = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        let entities = world.insert(shared, components.clone()).to_vec();

        let first = *entities.first().unwrap();
        world.delete(first);

        for (i, e) in entities.iter().skip(1).enumerate() {
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i + 1).map(|(x, _)| x), Some(&x as &Pos)),
                None => assert_eq!(components.get(i + 1).map(|(x, _)| x), None),
            }
            match world.get_component(*e) {
                Some(x) => assert_eq!(components.get(i + 1).map(|(_, x)| x), Some(&x as &Rot)),
                None => assert_eq!(components.get(i + 1).map(|(_, x)| x), None),
            }
        }
    }
}
