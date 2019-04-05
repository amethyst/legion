use crate::*;
use downcast_rs::{impl_downcast, Downcast};
use fnv::{FnvHashMap, FnvHashSet};
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::mem::size_of;
use std::sync::atomic::AtomicIsize;
use std::sync::Arc;

impl_downcast!(ComponentStorage);
trait ComponentStorage: Downcast + Debug {
    fn remove(&mut self, id: ComponentIndex);
    fn fetch_remove(
        &mut self,
        id: ComponentIndex,
    ) -> (TypeId, Box<dyn Fn(&mut ChunkBuilder)>, Box<dyn ChunkInit>);
    fn len(&self) -> usize;
}

#[derive(Debug)]
struct StorageVec<T: Component> {
    version: UnsafeCell<Wrapping<usize>>,
    data: UnsafeCell<Vec<T>>,
}

impl<T: Component> StorageVec<T> {
    fn with_capacity(capacity: usize) -> Self {
        StorageVec {
            version: UnsafeCell::new(Wrapping(0)),
            data: UnsafeCell::new(Vec::<T>::with_capacity(capacity)),
        }
    }

    fn version(&self) -> usize {
        unsafe { (*self.version.get()).0 }
    }

    unsafe fn data(&self) -> &Vec<T> {
        &(*self.data.get())
    }

    unsafe fn data_mut(&self) -> &mut Vec<T> {
        *self.version.get() += Wrapping(1);
        &mut (*self.data.get())
    }
}

impl<T: Component> ComponentStorage for StorageVec<T> {
    fn remove(&mut self, id: ComponentIndex) {
        unsafe {
            self.data_mut().swap_remove(id as usize);
        }
    }

    fn fetch_remove(
        &mut self,
        id: ComponentIndex,
    ) -> (TypeId, Box<dyn Fn(&mut ChunkBuilder)>, Box<dyn ChunkInit>) {
        let component = unsafe { self.data_mut().swap_remove(id as usize) };
        DynamicSingleEntitySource::component_tuple(component)
    }

    fn len(&self) -> usize {
        unsafe { self.data().len() }
    }
}

impl_downcast!(TagValue);
pub trait TagValue: Downcast + Sync + Send + Debug + 'static {
    fn downcast_equals(&self, _: &TagValue) -> bool;
}

/// Raw unsafe storage for components associated with entities.
///
/// All entities contained within a chunk have the same shared data values and entity data types.
///
/// Data slices obtained from a chunk when indexed with a given index all refer to the same entity.
#[derive(Debug)]
pub struct Chunk {
    id: ChunkId,
    capacity: usize,
    entities: StorageVec<Entity>,
    components: FnvHashMap<TypeId, Box<dyn ComponentStorage>>,
    tags: FnvHashMap<TypeId, Arc<dyn TagValue>>,
    borrows: FnvHashMap<TypeId, AtomicIsize>,
}

unsafe impl Sync for Chunk {}

impl Chunk {
    /// Gets the ID of the chunk.
    pub fn id(&self) -> ChunkId {
        self.id
    }

    /// Gets the number of entities stored within the chunk.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Determines if the chunk has reached capacity and can no longer accept more entities.
    pub fn is_full(&self) -> bool {
        self.len() == self.capacity
    }

    /// Gets a slice of `Entity` IDs of entities contained within the chunk.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is writing into
    /// the chunk's entities vector before calling this function.
    pub unsafe fn entities(&self) -> &[Entity] {
        self.entities.data()
    }

    /// Gets a mutable vector of entity IDs contained within the chunk.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is reading or writing
    /// the chunk's entities vector before calling this function.
    pub unsafe fn entities_unchecked(&self) -> &mut Vec<Entity> {
        self.entities.data_mut()
    }

    /// Gets a vector of component data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is writing to
    /// this component type in the chunk before calling this function.
    pub unsafe fn components_unchecked<T: Component>(&self) -> Option<&Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data())
    }

    /// Gets a mutable vector of component data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is reaing or writing to
    /// this component type in the chunk before calling this function.
    pub unsafe fn components_mut_unchecked<T: Component>(&self) -> Option<&mut Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data_mut())
    }

    /// Gets a slice of component data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Panics
    ///
    /// This function performs runtime borrow checking. It will panic if other code is borrowing
    /// the same component type mutably.
    pub fn components<'a, T: Component>(&'a self) -> Option<BorrowedSlice<'a, T>> {
        match unsafe { self.components_unchecked() } {
            Some(data) => {
                let borrow = self.borrow::<T>();
                Some(BorrowedSlice::new(data, borrow))
            }
            None => None,
        }
    }

    /// Gets a mutable slice of component data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Panics
    ///
    /// This function performs runtime borrow checking. It will panic if other code is borrowing
    /// the same component type.
    pub fn components_mut<'a, T: Component>(&'a self) -> Option<BorrowedMutSlice<'a, T>> {
        match unsafe { self.components_mut_unchecked() } {
            Some(data) => {
                let borrow = self.borrow_mut::<T>();
                Some(BorrowedMutSlice::new(data, borrow))
            }
            None => None,
        }
    }

    /// Gets the version number of a given component type.
    ///
    /// Each component array in the slice has a version number which is
    /// automatically incremented every time it is retrieved mutably.
    pub fn component_version<T: Component>(&self) -> Option<usize> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.version())
    }

    /// Gets a tag value associated with all entities in the chunk.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    pub fn tag<T: Tag>(&self) -> Option<&T> {
        self.tags
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<T>())
    }

    /// Removes an entity from the chunk.
    ///
    /// Returns the ID of any entity which was swapped into the location of the
    /// removed entity.
    pub fn remove(&mut self, id: ComponentIndex) -> Option<Entity> {
        unsafe {
            let index = id as usize;
            self.entities.data_mut().swap_remove(index);
            for storage in self.components.values_mut() {
                storage.remove(id);
            }

            if self.entities.len() > index {
                Some(*self.entities.data().get(index).unwrap())
            } else {
                None
            }
        }
    }

    /// Removes and entity from the chunk and returns a dynamic tag set and entity source
    /// which can be used to re-insert the removed entity into a world.
    ///
    /// Returns the ID of any entity which was swapped into the location of the
    /// removed entity.
    pub fn fetch_remove(
        &mut self,
        id: ComponentIndex,
    ) -> (Option<Entity>, DynamicTagSet, DynamicSingleEntitySource) {
        unsafe {
            let index = id as usize;
            let entity = self.entities.data_mut().swap_remove(index);
            let components = self.components.values_mut().map(|s| s.fetch_remove(id));

            let tags = DynamicTagSet {
                tags: self.tags.clone(),
            };

            let components = DynamicSingleEntitySource {
                entity,
                components: components.collect(),
            };

            let moved = if self.entities.len() > index {
                Some(*self.entities.data().get(index).unwrap())
            } else {
                None
            };

            (moved, tags, components)
        }
    }

    /// Validates that the chunk contains a balanced number of components and entities.
    pub fn validate(&self) {
        let valid = self
            .components
            .values()
            .fold(true, |total, s| total && s.len() == self.entities.len());
        assert!(valid, "imbalanced chunk components");
    }

    fn borrow<'a, T: Component>(&'a self) -> Borrow<'a> {
        let id = TypeId::of::<T>();
        let state = self
            .borrows
            .get(&id)
            .expect("entity data type not found in chunk");
        Borrow::aquire_read(state).unwrap()
    }

    fn borrow_mut<'a, T: Component>(&'a self) -> Borrow<'a> {
        let id = TypeId::of::<T>();
        let state = self
            .borrows
            .get(&id)
            .expect("entity data type not found in chunk");
        Borrow::aquire_write(state).unwrap()
    }
}

/// Constructs a new `Chunk`.
pub struct ChunkBuilder {
    components: Vec<(
        TypeId,
        usize,
        Box<dyn FnMut(usize) -> Box<dyn ComponentStorage>>,
    )>,
    tags: FnvHashMap<TypeId, Arc<dyn TagValue>>,
}

impl ChunkBuilder {
    const MAX_SIZE: usize = 16 * 1024;

    /// Constructs a new `ChunkBuilder`.
    pub fn new() -> ChunkBuilder {
        ChunkBuilder {
            components: Vec::new(),
            tags: FnvHashMap::default(),
        }
    }

    /// Registers an entity data component type.
    pub fn register_component<T: Component>(&mut self) {
        let constructor = |capacity| {
            Box::new(StorageVec::<T>::with_capacity(capacity)) as Box<dyn ComponentStorage>
        };
        self.components
            .push((TypeId::of::<T>(), size_of::<T>(), Box::new(constructor)));
    }

    /// Registers a tag type.
    pub fn register_tag<T: Tag>(&mut self, data: Arc<T>) {
        self.tags
            .insert(TypeId::of::<T>(), data as Arc<dyn TagValue>);
    }

    pub fn register_tags<I>(&mut self, tags: I)
    where
        I: IntoIterator<Item = (TypeId, Arc<dyn TagValue>)>,
    {
        self.tags.extend(tags);
    }

    /// Builds a new `Chunk`.
    pub fn build(self, id: ChunkId) -> Chunk {
        let size_bytes = *self
            .components
            .iter()
            .map(|(_, size, _)| size)
            .max()
            .unwrap_or(&ChunkBuilder::MAX_SIZE);
        let capacity = std::cmp::max(1, ChunkBuilder::MAX_SIZE / size_bytes);
        Chunk {
            id,
            capacity: capacity,
            borrows: self
                .components
                .iter()
                .map(|(id, _, _)| (*id, AtomicIsize::new(0)))
                .collect(),
            entities: StorageVec::with_capacity(capacity),
            components: self
                .components
                .into_iter()
                .map(|(id, _, mut con)| (id, con(capacity)))
                .collect(),
            tags: self.tags,
        }
    }
}

pub struct DynamicTagSet {
    tags: FnvHashMap<TypeId, Arc<dyn TagValue>>,
}

impl DynamicTagSet {
    pub fn set_tag<T: Tag>(&mut self, tag: Arc<T>) {
        self.tags.insert(TypeId::of::<T>(), tag);
    }

    pub fn remove_tag<T: Tag>(&mut self) -> bool {
        self.tags.remove(&TypeId::of::<T>()).is_some()
    }
}

impl TagSet for DynamicTagSet {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool {
        archetype.tags.len() == self.tags.len()
            && self.tags.keys().all(|k| archetype.tags.contains(k))
    }

    fn is_chunk_match(&self, chunk: &Chunk) -> bool {
        self.tags
            .iter()
            .all(|(k, v)| chunk.tags.get(k).unwrap().downcast_equals(v.as_ref()))
    }

    fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
        chunk.register_tags(self.tags.iter().map(|(k, v)| (*k, v.clone())));
    }

    fn types(&self) -> FnvHashSet<TypeId> {
        self.tags.keys().map(|id| *id).collect()
    }
}

trait ChunkInit: Send {
    fn call(self: Box<Self>, chunk: &mut Chunk);
}

impl<'a, F: FnOnce(&mut Chunk) + Send> ChunkInit for F {
    fn call(self: Box<Self>, chunk: &mut Chunk) {
        (*self)(chunk);
    }
}

pub struct DynamicSingleEntitySource {
    entity: Entity,
    components: Vec<(TypeId, Box<dyn Fn(&mut ChunkBuilder)>, Box<dyn ChunkInit>)>,
}

impl DynamicSingleEntitySource {
    fn component_tuple<T: Component>(
        component: T,
    ) -> (TypeId, Box<dyn Fn(&mut ChunkBuilder)>, Box<dyn ChunkInit>) {
        let chunk_setup = |chunk: &mut ChunkBuilder| chunk.register_component::<T>();

        let data_initializer = |chunk: &mut Chunk| unsafe {
            chunk.components_mut_unchecked().unwrap().push(component)
        };

        (
            TypeId::of::<T>(),
            Box::new(chunk_setup),
            Box::new(data_initializer),
        )
    }

    pub fn add_component<T: Component>(&mut self, component: T) {
        self.remove_component::<T>();
        self.components.push(Self::component_tuple(component));
    }

    pub fn remove_component<T: Component>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        if let Some(i) = self
            .components
            .iter()
            .enumerate()
            .filter(|(_, (id, _, _))| id == &type_id)
            .map(|(i, _)| i)
            .next()
        {
            self.components.remove(i);
            true
        } else {
            false
        }
    }
}

impl EntitySource for DynamicSingleEntitySource {
    fn is_archetype_match(&self, archetype: &Archetype) -> bool {
        archetype.components.len() == self.components.len()
            && self
                .components
                .iter()
                .all(|(id, _, _)| archetype.components.contains(&id))
    }

    fn configure_chunk(&self, chunk: &mut ChunkBuilder) {
        for (_, f, _) in self.components.iter() {
            f(chunk);
        }
    }

    fn types(&self) -> FnvHashSet<TypeId> {
        self.components.iter().map(|(id, _, _)| *id).collect()
    }

    fn is_empty(&mut self) -> bool {
        self.components.len() == 0
    }

    fn write<'a>(&mut self, chunk: &'a mut Chunk, _: &mut EntityAllocator) -> usize {
        if !chunk.is_full() {
            unsafe {
                chunk.entities_unchecked().push(self.entity);

                for (_, _, f) in self.components.drain(..) {
                    f.call(chunk);
                }
            }

            1
        } else {
            0
        }
    }
}

/// Stores all chunks with a given data layout.
pub struct Archetype {
    id: ArchetypeId,
    logger: slog::Logger,
    next_chunk_id: u16,
    version: u16,
    /// The entity data component types that all chunks contain.
    pub components: FnvHashSet<TypeId>,
    /// The tag types that all chunks contains.
    pub tags: FnvHashSet<TypeId>,
    /// The chunks that belong to this archetype.
    pub chunks: Vec<Chunk>,
}

impl Archetype {
    /// Constructs a new `Archetype`.
    pub fn new(
        id: ArchetypeId,
        logger: slog::Logger,
        components: FnvHashSet<TypeId>,
        tags: FnvHashSet<TypeId>,
    ) -> Archetype {
        Archetype {
            id,
            logger,
            next_chunk_id: 0,
            version: 0,
            components,
            tags,
            chunks: Vec::new(),
        }
    }

    /// Gets the archetype ID.
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Gets the archetype version.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Gets a chunk reference.
    pub fn chunk(&self, id: ChunkIndex) -> Option<&Chunk> {
        self.chunks.get(id as usize)
    }

    /// Gets a mutable chunk reference.
    pub fn chunk_mut(&mut self, id: ChunkIndex) -> Option<&mut Chunk> {
        self.chunks.get_mut(id as usize)
    }

    /// Determines if the archetype's chunks contain the given entity data component type.
    pub fn has_component<T: Component>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    /// Determines if the archetype's chunks contain the given tag type.
    pub fn has_tag<T: Tag>(&self) -> bool {
        self.tags.contains(&TypeId::of::<T>())
    }

    /// Gets a slice reference of chunks.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// Finds a chunk which is suitable for the given data sources, or constructs a new one.
    pub fn get_or_create_chunk<'a, 'b, 'c, S: TagSet, C: EntitySource>(
        &'a mut self,
        tags: &'b S,
        components: &'c C,
    ) -> (ChunkIndex, &'a mut Chunk) {
        match self
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_full() && tags.is_chunk_match(c))
            .map(|(i, _)| i)
            .next()
        {
            Some(i) => (i as ChunkIndex, unsafe { self.chunks.get_unchecked_mut(i) }),
            None => {
                let mut builder = ChunkBuilder::new();
                tags.configure_chunk(&mut builder);
                components.configure_chunk(&mut builder);

                let chunk_id = self.id.chunk(self.next_chunk_id);
                let chunk_index = self.chunks.len() as ChunkIndex;
                self.next_chunk_id += 1;
                self.chunks.push(builder.build(chunk_id));
                self.version += 1;

                let chunk = self.chunks.last_mut().unwrap();

                debug!(self.logger, "allocated chunk"; "chunk_id" => chunk_id.id);

                (chunk_index, chunk)
            }
        }
    }
}
