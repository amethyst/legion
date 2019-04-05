use crate::borrows::*;
use crate::ArchetypeIndex;
use crate::ChunkIndex;
use crate::ComponentIndex;
use crate::Entity;
use crate::EntityAllocator;
use crate::WorldId;
use downcast_rs::{impl_downcast, Downcast};
use slog::Logger;
use slog::{debug, o, trace, Drain};
use smallvec::SmallVec;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::iter::Peekable;
use std::slice::Iter;
use std::sync::atomic::AtomicIsize;

const CHUNK_MAX_SIZE: usize = 16 * 1024;

pub trait Tag: TagValue + PartialEq + Clone {}

impl<T: TagValue + PartialEq + Clone> Tag for T {}

pub trait Component: Send + Sync + Debug + 'static {}

impl<T: Send + Sync + Debug + 'static> Component for T {}

impl_downcast!(TagValue);
pub trait TagValue: Downcast + Sync + Send + Debug + 'static {}

impl<T: Downcast + Sync + Send + Debug + 'static> TagValue for T {}

pub struct EntityLocation {
    archetype_index: usize,
    chunk_index: usize,
    component_index: usize,
}

pub struct Storage {
    logger: Logger,
    world_id: WorldId,
    component_types: Vec<SmallVec<[TypeId; 5]>>,
    tag_types: Vec<SmallVec<[TypeId; 3]>>,
    chunk_sets: Vec<ChunkSet>,
    chunk_constructors: Vec<Box<dyn ChunkConstructor>>,
}

impl Storage {
    pub fn new<L: Into<Option<Logger>>>(world: WorldId, logger: L) -> Self {
        Storage {
            world_id: world,
            logger: logger.into().unwrap_or(Logger::root(
                slog_stdlog::StdLog.fuse(),
                o!("world_id" => world.id),
            )),
            component_types: Vec::new(),
            tag_types: Vec::new(),
            chunk_sets: Vec::new(),
            chunk_constructors: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.chunk_sets.len()
    }

    pub fn insert<T, C>(&mut self, entities: &mut EntityAllocator, mut tags: T, components: C)
    where
        T: TagSet,
        C: IntoComponentSource,
    {
        let mut components = components.into();
        let archetype = self.find_or_create_archetype(&mut tags, &mut components);
        assert!(self.len() >= archetype);

        while !components.is_empty() {
            let chunk = self.find_or_create_chunk(archetype, &mut tags);
            let chunk_components = unsafe {
                self.chunk_sets
                    .get_unchecked_mut(archetype)
                    .components
                    .get_unchecked_mut(chunk)
            };

            let allocated = components.write(chunk_components, entities);
            trace!(
                self.logger,
                "pushed {count} entities into chunk",
                count = allocated;
                "world_id" => self.world_id.id,
                "archetype_id" => archetype,
                "chunk_id" => chunk
            );

            let start = chunk_components.len() - allocated;
            let added = chunk_components.entities().iter().enumerate().skip(start);
            for (i, e) in added {
                let location = (
                    archetype as ArchetypeIndex,
                    chunk as ChunkIndex,
                    i as ComponentIndex,
                );
                entities.set_location(&e.index, location);
            }
        }
    }

    pub fn get_component<'a, T: Component>(
        &'a self,
        entity: &EntityLocation,
    ) -> Option<Borrowed<'a, T>> {
        self.chunk_sets
            .get(entity.archetype_index)
            .and_then(|c| c.components.get(entity.chunk_index))
            .and_then(|c| c.components())
            .and_then(|c| c.single(entity.component_index))
    }

    pub fn get_component_mut<'a, T: Component>(
        &'a self,
        entity: &EntityLocation,
    ) -> Option<BorrowedMut<'a, T>> {
        self.chunk_sets
            .get(entity.archetype_index)
            .and_then(|c| c.components.get(entity.chunk_index))
            .and_then(|c| c.components_mut())
            .and_then(|c| c.single(entity.component_index))
    }

    pub fn get_tag<T: Tag>(&self, entity: &EntityLocation) -> Option<&T> {
        self.chunk_sets
            .get(entity.archetype_index)
            .and_then(|c| c.tags())
            .and_then(|t| t.get(entity.chunk_index))
    }

    fn push(
        &mut self,
        arch_components: SmallVec<[TypeId; 5]>,
        arch_tags: SmallVec<[TypeId; 3]>,
        arch_chunks: ChunkSet,
        chunk_constructor: Box<dyn ChunkConstructor>,
    ) -> usize {
        self.component_types.push(arch_components);
        self.tag_types.push(arch_tags);
        self.chunk_sets.push(arch_chunks);
        self.chunk_constructors.push(chunk_constructor);

        debug!(
            self.logger,
            "created archetype";
            "archetype_id" => (self.len() - 1)
        );

        self.len() - 1
    }

    fn find_or_create_archetype<T, C>(&mut self, tags: &mut T, components: &mut C) -> usize
    where
        T: TagLayout,
        C: ComponentLayout,
    {
        {
            let archetype_data = ArchetypeData {
                component_types: &self.component_types,
                tag_types: &self.tag_types,
            };

            let tag_iter = tags.collect(&archetype_data);
            let tag_matches =
                tag_iter.map(|x| <T as Filter<'_, ArchetypeData<'_>>>::is_match(tags, x));
            let component_iter = components.collect(&archetype_data);
            let component_matches = component_iter.map(|x| components.is_match(x));
            if let Some(i) = tag_matches
                .zip(component_matches)
                .enumerate()
                .filter(|(_, (t, c))| *t && *c)
                .map(|(i, _)| i)
                .next()
            {
                return i;
            };
        }

        let (tag_types, tag_storage) = (tags.tag_types(), tags.construct());
        let (comp_types, comp_constructor) = (components.component_types(), components.construct());
        let arch_tags = SmallVec::from_slice(&tag_types);
        let arch_chunks = ChunkSet::new(tag_storage);
        let arch_components = SmallVec::from_slice(&comp_types);

        self.push(arch_components, arch_tags, arch_chunks, comp_constructor)
    }

    fn find_or_create_chunk<T>(&mut self, archetype: usize, tags: &mut T) -> usize
    where
        T: TagSet,
    {
        assert!(self.len() >= archetype, "archetype index out of bounds");
        let chunk_set = unsafe { self.chunk_sets.get_unchecked_mut(archetype) };

        let index = {
            let chunk_data = ChunkData {
                chunk_set: &chunk_set,
            };

            let chunk_iter = tags.collect(&chunk_data);
            let index = chunk_iter
                .map(|x| <T as Filter<'_, ChunkData<'_>>>::is_match(tags, x))
                .zip(chunk_set.components().iter())
                .enumerate()
                .filter(|(_, (matches, components))| *matches && !components.is_full())
                .map(|(i, _)| i)
                .next();

            match index {
                Some(i) => i,
                None => {
                    let constructor = unsafe { self.chunk_constructors.get_unchecked(archetype) };
                    let components = constructor.construct();
                    let tags = tags.tags();
                    chunk_set.push(tags, components);

                    debug!(
                        self.logger,
                        "created chunk";
                        "archetype_id" => archetype,
                        "chunk_id" => (chunk_set.len() - 1)
                    );

                    chunk_set.len() - 1
                }
            }
        };

        index
    }
}

pub struct ChunkSet {
    tags: SmallVec<[(TypeId, Box<dyn TagVec>); 3]>,
    components: Vec<ComponentSet>,
}

impl ChunkSet {
    pub fn new<I>(tag_storage: I) -> Self
    where
        I: IntoIterator<Item = (TypeId, Box<dyn TagVec>)>,
    {
        ChunkSet {
            tags: tag_storage.into_iter().collect(),
            components: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn components(&self) -> &[ComponentSet] {
        &self.components
    }

    pub fn tags<T: Tag>(&self) -> Option<&[T]> {
        let type_id = TypeId::of::<T>();
        self.tags
            .iter()
            .filter(|(t, _)| t == &type_id)
            .next()
            .and_then(|(_, v)| v.as_ref().downcast_ref::<Vec<T>>())
            .map(|v| v.as_slice())
    }

    pub fn push(&mut self, tags: Vec<(TypeId, Box<dyn TagValue>)>, components: ComponentSet) {
        for (tag_type, tag_value) in tags {
            let vec = self
                .tags
                .iter_mut()
                .filter(|(t, _)| t == &tag_type)
                .map(|(_, v)| v)
                .next()
                .unwrap();
            vec.push(tag_value);
        }

        self.components.push(components);
    }
}

impl_downcast!(TagVec);
pub trait TagVec: Downcast + Debug {
    fn push(&mut self, value: Box<dyn TagValue>);
}

impl<T: Tag> TagVec for Vec<T> {
    fn push(&mut self, value: Box<dyn TagValue>) {
        let value = value.downcast::<T>().unwrap();
        self.push(*value);
    }
}

impl_downcast!(ComponentVec);
pub trait ComponentVec: Downcast {}

impl<T: Component> ComponentVec for Vec<T> {}

pub trait ChunkConstructor {
    fn construct(&self) -> ComponentSet;
}

impl<F> ChunkConstructor for F
where
    F: Fn() -> ComponentSet,
{
    fn construct(&self) -> ComponentSet {
        (*self)()
    }
}

pub struct ComponentSet {
    entities: UnsafeCell<Vec<Entity>>,
    components: UnsafeCell<SmallVec<[(TypeId, AtomicIsize, Box<dyn ComponentVec>); 5]>>,
}

impl ComponentSet {
    fn new<C: Iterator<Item = (TypeId, Box<dyn ComponentVec>)>>(
        capacity: usize,
        components: C,
    ) -> Self {
        ComponentSet {
            entities: UnsafeCell::new(Vec::with_capacity(capacity)),
            components: UnsafeCell::new(
                components
                    .map(|(t, v)| (t, AtomicIsize::new(0), v))
                    .collect(),
            ),
        }
    }

    fn is_full(&self) -> bool {
        self.entities().len() >= self.entities().capacity()
    }

    fn len(&self) -> usize {
        self.entities().len()
    }

    fn entities(&self) -> &Vec<Entity> {
        unsafe { &*self.entities.get() }
    }

    fn components<'a, T: Component>(&'a self) -> Option<BorrowedSlice<'a, T>> {
        let type_id = TypeId::of::<T>();
        unsafe {
            (&*self.components.get())
                .iter()
                .filter(|(t, _, _)| t == &type_id)
                .next()
                .map(|(_, b, v)| {
                    (
                        Borrow::aquire_read(b).unwrap(),
                        v.as_ref().downcast_ref::<Vec<T>>(),
                    )
                })
                .and_then(|(b, v)| v.map(|v| BorrowedSlice::new(v, b)))
        }
    }

    fn components_mut<'a, T: Component>(&'a self) -> Option<BorrowedMutSlice<'a, T>> {
        let type_id = TypeId::of::<T>();
        unsafe {
            (&mut *self.components.get())
                .iter_mut()
                .filter(|(t, _, _)| t == &type_id)
                .next()
                .map(|(_, b, v)| {
                    (
                        Borrow::aquire_write(b).unwrap(),
                        v.as_mut().downcast_mut::<Vec<T>>(),
                    )
                })
                .and_then(|(b, v)| v.map(|v| BorrowedMutSlice::new(v, b)))
        }
    }

    fn mutate_unsafe<'a>(&'a mut self) -> ComponentSetUnsafeAccessor<'a> {
        ComponentSetUnsafeAccessor { components: &*self }
    }
}

pub struct ComponentSetUnsafeAccessor<'a> {
    components: &'a ComponentSet,
}

impl<'a> ComponentSetUnsafeAccessor<'a> {
    unsafe fn is_full(&self) -> bool {
        self.components.is_full()
    }

    unsafe fn entities_mut(&self) -> &mut Vec<Entity> {
        &mut *self.components.entities.get()
    }

    unsafe fn components_mut<T: Component>(&self) -> Option<&mut Vec<T>> {
        let type_id = TypeId::of::<T>();
        (&mut *self.components.components.get())
            .iter_mut()
            .filter(|(t, _, _)| t == &type_id)
            .next()
            .and_then(|(_, _, v)| v.as_mut().downcast_mut::<Vec<T>>())
    }
}

pub trait Filter<'a, T> {
    type Iter: Iterator;

    fn collect(&self, source: &'a T) -> Self::Iter;
    fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> bool;
}

pub struct ArchetypeData<'a> {
    component_types: &'a Vec<SmallVec<[TypeId; 5]>>,
    tag_types: &'a Vec<SmallVec<[TypeId; 3]>>,
}

pub struct ChunkData<'a> {
    chunk_set: &'a ChunkSet,
}

pub trait TagLayout: Sized + for<'a> Filter<'a, ArchetypeData<'a>> {
    fn tag_types(&self) -> Vec<TypeId>; //todo change to &[TypeId];
    fn construct(&self) -> Vec<(TypeId, Box<dyn TagVec>)>;
}

pub trait ComponentLayout: Sized + for<'a> Filter<'a, ArchetypeData<'a>> {
    fn component_types(&self) -> Vec<TypeId>; //todo change to &[TypeId];
    fn construct(&self) -> Box<dyn ChunkConstructor>;
    fn per_chunk() -> usize;
}

pub trait TagSet: TagLayout + for<'a> Filter<'a, ChunkData<'a>> {
    fn tags(&self) -> Vec<(TypeId, Box<dyn TagValue>)>;
}

pub trait ComponentSource: ComponentLayout {
    fn is_empty(&mut self) -> bool;
    fn write(&mut self, chunk: &mut ComponentSet, allocator: &mut EntityAllocator) -> usize;
}

pub trait IntoComponentSource {
    type Source: ComponentSource;

    fn into(self) -> Self::Source;
}

// impl<T: ComponentSource> IntoComponentSource for T {
//     default type Source = Self;

//     default fn into(self) -> Self::Source {
//         self
//     }
// }

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
    use itertools::Zip;
    use std::iter::Repeat;
    use std::iter::Take;

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
                fn component_types(&self) -> Vec<TypeId> {
                    let mut v = vec![$( TypeId::of::<$ty>() ),*];
                    v.sort();
                    v
                }

                fn construct(&self) -> Box<dyn ChunkConstructor> {
                    let capacity = Self::per_chunk();
                    Box::new(move ||
                        ComponentSet::new(capacity, vec![
                            $(
                                (
                                    TypeId::of::<$ty>(),
                                    Box::new(Vec::<$ty>::with_capacity(capacity)) as Box<dyn ComponentVec>
                                )
                            ),*
                        ].into_iter())
                    )
                }

                fn per_chunk() -> usize {
                    let sizes = &[
                        $( std::mem::size_of::<$ty>() ),*
                    ];
                    let size_bytes = sizes
                        .iter()
                        .max()
                        .unwrap_or(&std::mem::size_of::<Entity>());
                    std::cmp::max(1, CHUNK_MAX_SIZE / size_bytes)
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

                fn write(&mut self, components: &mut ComponentSet, allocator: &mut EntityAllocator) -> usize {
                    #![allow(non_snake_case)]
                    let mutator = components.mutate_unsafe();
                    let mut count = 0;

                    unsafe {
                        let entities = mutator.entities_mut();
                        $(
                            let $ty = mutator.components_mut::<$ty>().unwrap();
                        )*

                        while let Some(($( $id, )*)) = { if mutator.is_full() { None } else { self.iter.next() } } {
                            let entity = allocator.create_entity();
                            entities.push(entity);
                            $(
                                $ty.push($id);
                            )*
                            count += 1;
                        }
                    }

                    count
                }
            }

            impl<'a, I, $( $ty ),*> Filter<'a, ArchetypeData<'a>> for ComponentTupleSet<($( $ty, )*), I>
            where
                I: Iterator<Item = ($( $ty, )*)>,
                $( $ty: Component ),*
            {
                type Iter = Iter<'a, SmallVec<[TypeId; 5]>>;

                fn collect(&self, source: &'a ArchetypeData<'a>) -> Self::Iter {
                    source.component_types.iter()
                }

                fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> bool {
                    let types = &[$( TypeId::of::<$ty>() ),*];
                    //todo possible once ComponentLayout::component_types() is returning a sorted &[TypeId]
                    //types.len() == item.len() && types.iter().zip(item.iter()).all(|(x, y)| x == y)
                    types.len() == item.len() && types.iter().all(|t| item.contains(t))
                }
            }
        };
        ( @TAG_SET $( $ty: ident => $id: ident ),* ) => {
            impl_data_tuple!(@CHUNK_FILTER $( $ty => $id ),*);

            impl<$( $ty ),*> TagSet for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                fn tags(&self) -> Vec<(TypeId, Box<dyn TagValue>)> {
                    #![allow(non_snake_case)]
                    let ($($id,)*) = self;
                    vec![
                        $( (TypeId::of::<$ty>(), Box::new($id.clone())), )*
                    ]
                }
            }

            impl <$( $ty ),*> TagLayout for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                fn tag_types(&self) -> Vec<TypeId> {
                    let mut v = vec![$( TypeId::of::<$ty>() ),*];
                    v.sort();
                    v
                }

                fn construct(&self) -> Vec<(TypeId, Box<dyn TagVec>)> {
                    vec![
                        $( (TypeId::of::<$ty>(), Box::new(Vec::<$ty>::new()) as Box<dyn TagVec>) ),*
                    ]
                }
            }

            impl<'a, $( $ty ),*> Filter<'a, ArchetypeData<'a>> for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                type Iter = Iter<'a, SmallVec<[TypeId; 3]>>;

                fn collect(&self, source: &'a ArchetypeData<'a>) -> Self::Iter {
                    source.tag_types.iter()
                }

                fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> bool {
                    let types = &[$( TypeId::of::<$ty>() ),*];
                    //todo possible once TagLayout::tag_types() is returning a sorted &[TypeId]
                    //types.len() == item.len() && types.iter().zip(item.iter()).all(|(x, y)| x == y)
                    types.len() == item.len() && types.iter().all(|t| item.contains(t))
                }
            }
        };
        ( @CHUNK_FILTER $( $ty: ident => $id: ident ),+ ) => {
            impl<'a, $( $ty ),*> Filter<'a, ChunkData<'a>> for ($( $ty, )*)
            where
                $( $ty: Tag ),*
            {
                type Iter = Zip<($( Iter<'a, $ty>, )*)>;

                fn collect(&self, source: &'a ChunkData<'a>) -> Self::Iter {
                    let iters = (
                        $( source.chunk_set.tags
                            .iter()
                            .filter(|(t, _)| t == &TypeId::of::<$ty>())
                            .map(|(_, v)| v.as_ref().downcast_ref::<Vec<$ty>>().unwrap().iter())
                            .next()
                            .unwrap(),
                        )*
                    );

                    itertools::multizip(iters)
                }

                fn is_match(&mut self, item: <Self::Iter as Iterator>::Item) -> bool {
                    #![allow(non_snake_case)]
                    let ($( $ty, )*) = self;
                    ($( &*$ty, )*) == item
                }
            }
        };
        ( @CHUNK_FILTER ) => {
            impl<'a> Filter<'a, ChunkData<'a>> for () {
                type Iter = Take<Repeat<()>>;

                fn collect(&self, source: &'a ChunkData<'a>) -> Self::Iter {
                    std::iter::repeat(()).take(source.chunk_set.len())
                }

                fn is_match(&mut self, _: <Self::Iter as Iterator>::Item) -> bool {
                    true
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
    use crate::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

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

    fn create() -> (EntityAllocator, Storage) {
        (
            EntityAllocator::new(Arc::from(Mutex::new(BlockAllocator::new()))),
            Storage::new(WorldId::new(0), None),
        )
    }

    #[test]
    fn insert() {
        let (mut allocator, mut storage) = create();

        let shared = (1usize, 2f32, 3u16);
        let components = vec![(4f32, 5u64, 6u16), (4f32, 5u64, 6u16)];
        storage.insert(&mut allocator, shared, components);

        assert_eq!(2, allocator.allocation_buffer().len());
    }

    #[test]
    fn get_component() {
        let (mut allocator, mut storage) = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        storage.insert(&mut allocator, shared, components.clone());

        for (i, e) in allocator.allocation_buffer().iter().enumerate() {
            let location = allocator
                .get_location(&e.index)
                .map(|(arch, chunk, comp)| EntityLocation {
                    archetype_index: arch as usize,
                    chunk_index: chunk as usize,
                    component_index: comp as usize,
                })
                .unwrap();
            match storage.get_component(&location) {
                Some(x) => assert_eq!(components.get(i).map(|(x, _)| x), Some(&x as &Pos)),
                None => assert_eq!(components.get(i).map(|(x, _)| x), None),
            }
            match storage.get_component(&location) {
                Some(x) => assert_eq!(components.get(i).map(|(_, x)| x), Some(&x as &Rot)),
                None => assert_eq!(components.get(i).map(|(_, x)| x), None),
            }
        }
    }

    #[test]
    fn get_component_wrong_type() {
        let (mut allocator, mut storage) = create();

        storage.insert(&mut allocator, (), vec![(0f64,)]);

        let location = allocator
            .get_location(&allocator.allocation_buffer().get(0).unwrap().index)
            .map(|(arch, chunk, comp)| EntityLocation {
                archetype_index: arch as usize,
                chunk_index: chunk as usize,
                component_index: comp as usize,
            })
            .unwrap();

        assert_eq!(None, storage.get_component::<i32>(&location));
    }

    #[test]
    fn get_shared() {
        let (mut allocator, mut storage) = create();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6)),
        ];

        storage.insert(&mut allocator, shared, components);

        for e in allocator.allocation_buffer().iter() {
            let location = allocator
                .get_location(&e.index)
                .map(|(arch, chunk, comp)| EntityLocation {
                    archetype_index: arch as usize,
                    chunk_index: chunk as usize,
                    component_index: comp as usize,
                })
                .unwrap();
            assert_eq!(Some(&Static), storage.get_tag(&location));
            assert_eq!(Some(&Model(5)), storage.get_tag(&location));
        }
    }

    #[test]
    fn get_shared_wrong_type() {
        let (mut allocator, mut storage) = create();

        storage.insert(&mut allocator, (Static,), vec![(0f64,)]);

        let location = allocator
            .get_location(&allocator.allocation_buffer().get(0).unwrap().index)
            .map(|(arch, chunk, comp)| EntityLocation {
                archetype_index: arch as usize,
                chunk_index: chunk as usize,
                component_index: comp as usize,
            })
            .unwrap();

        assert_eq!(None, storage.get_tag::<Model>(&location));
    }
}
