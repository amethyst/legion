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
    fn len(&self) -> usize;
}

#[derive(Debug)]
struct StorageVec<T> {
    version: UnsafeCell<Wrapping<usize>>,
    data: UnsafeCell<Vec<T>>,
}

impl<T: Debug> StorageVec<T> {
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

impl<T: Debug + 'static> ComponentStorage for StorageVec<T> {
    fn remove(&mut self, id: ComponentIndex) {
        unsafe {
            self.data_mut().swap_remove(id as usize);
        }
    }

    fn len(&self) -> usize {
        unsafe { self.data().len() }
    }
}

impl_downcast!(SharedComponentStorage);
trait SharedComponentStorage: Downcast + Debug {}

#[derive(Debug)]
struct SharedComponentStore<T>(UnsafeCell<T>);

impl<T: SharedData> SharedComponentStorage for SharedComponentStore<T> {}

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
    shared: FnvHashMap<TypeId, Arc<dyn SharedComponentStorage>>,
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

    /// Gets a vector of entity data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is writing to
    /// this component type in the chunk before calling this function.
    pub unsafe fn entity_data_unchecked<T: EntityData>(&self) -> Option<&Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data())
    }

    /// Gets a mutable vector of entity data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Safety
    ///
    /// This function bypasses any borrow checking. Ensure no other code is reaing or writing to
    /// this component type in the chunk before calling this function.
    pub unsafe fn entity_data_mut_unchecked<T: EntityData>(&self) -> Option<&mut Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data_mut())
    }

    /// Gets a slice of entity data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Panics
    ///
    /// This function performs runtime borrow checking. It will panic if other code is borrowing
    /// the same component type mutably.
    pub fn entity_data<'a, T: EntityData>(&'a self) -> Option<BorrowedSlice<'a, T>> {
        match unsafe { self.entity_data_unchecked() } {
            Some(data) => {
                let borrow = self.borrow::<T>();
                Some(BorrowedSlice::new(data, borrow))
            }
            None => None,
        }
    }

    /// Gets a mutable slice of entity data.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    ///
    /// # Panics
    ///
    /// This function performs runtime borrow checking. It will panic if other code is borrowing
    /// the same component type.
    pub fn entity_data_mut<'a, T: EntityData>(&'a self) -> Option<BorrowedMutSlice<'a, T>> {
        match unsafe { self.entity_data_mut_unchecked() } {
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
    pub fn entity_data_version<T: EntityData>(&self) -> Option<usize> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.version())
    }

    /// Gets a shared data value associated with all entities in the chunk.
    ///
    /// Returns `None` if the chunk does not contain the requested data type.
    pub fn shared_data<T: SharedData>(&self) -> Option<&T> {
        unsafe {
            self.shared
                .get(&TypeId::of::<T>())
                .and_then(|s| s.downcast_ref::<SharedComponentStore<T>>())
                .map(|s| &*s.0.get())
        }
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

    /// Validates that the chunk contains a balanced number of components and entities.
    pub fn validate(&self) {
        let valid = self
            .components
            .values()
            .fold(true, |total, s| total && s.len() == self.entities.len());
        if !valid {
            panic!("imbalanced chunk components");
        }
    }

    fn borrow<'a, T: EntityData>(&'a self) -> Borrow<'a> {
        let id = TypeId::of::<T>();
        let state = self
            .borrows
            .get(&id)
            .expect("entity data type not found in chunk");
        Borrow::aquire_read(state).unwrap()
    }

    fn borrow_mut<'a, T: EntityData>(&'a self) -> Borrow<'a> {
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
    shared: FnvHashMap<TypeId, Arc<dyn SharedComponentStorage>>,
}

impl ChunkBuilder {
    const MAX_SIZE: usize = 16 * 1024;

    /// Constructs a new `ChunkBuilder`.
    pub fn new() -> ChunkBuilder {
        ChunkBuilder {
            components: Vec::new(),
            shared: FnvHashMap::default(),
        }
    }

    /// Registers an entity data component type.
    pub fn register_component<T: EntityData>(&mut self) {
        let constructor = |capacity| {
            Box::new(StorageVec::<T>::with_capacity(capacity)) as Box<dyn ComponentStorage>
        };
        self.components
            .push((TypeId::of::<T>(), size_of::<T>(), Box::new(constructor)));
    }

    /// Registers a shared data component type.
    pub fn register_shared<T: SharedData>(&mut self, data: T) {
        self.shared.insert(
            TypeId::of::<T>(),
            Arc::new(SharedComponentStore(UnsafeCell::new(data)))
                as Arc<dyn SharedComponentStorage>,
        );
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
            shared: self.shared,
        }
    }
}

/// Stores all chunks with a given data layout.
#[derive(Debug)]
pub struct Archetype {
    id: ArchetypeId,
    logger: slog::Logger,
    next_chunk_id: u16,
    /// The entity data component types that all chunks contain.
    pub components: FnvHashSet<TypeId>,
    /// The shared data component types that all chunks contains.
    pub shared: FnvHashSet<TypeId>,
    /// The chunks that belong to this archetype.
    pub chunks: Vec<Chunk>,
}

impl Archetype {
    /// Constructs a new `Archetype`.
    pub fn new(
        id: ArchetypeId,
        logger: slog::Logger,
        components: FnvHashSet<TypeId>,
        shared: FnvHashSet<TypeId>,
    ) -> Archetype {
        Archetype {
            id,
            logger,
            next_chunk_id: 0,
            components,
            shared,
            chunks: Vec::new(),
        }
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
    pub fn has_component<T: EntityData>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    /// Determines if the archetype's chunks contain the given shared data component type.
    pub fn has_shared<T: SharedData>(&self) -> bool {
        self.shared.contains(&TypeId::of::<T>())
    }

    /// Gets a slice reference of chunks.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// Finds a chunk which is suitable for the given data sources, or constructs a new one.
    pub fn get_or_create_chunk<'a, 'b, 'c, S: SharedDataSet, C: EntitySource>(
        &'a mut self,
        shared: &'b S,
        components: &'c C,
    ) -> (ChunkIndex, &'a mut Chunk) {
        match self
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_full() && shared.is_chunk_match(c))
            .map(|(i, _)| i)
            .next()
        {
            Some(i) => (i as ChunkIndex, unsafe { self.chunks.get_unchecked_mut(i) }),
            None => {
                let mut builder = ChunkBuilder::new();
                shared.configure_chunk(&mut builder);
                components.configure_chunk(&mut builder);

                let chunk_id = self.id.chunk(self.next_chunk_id);
                let chunk_index = self.chunks.len() as ChunkIndex;
                self.next_chunk_id += 1;
                self.chunks.push(builder.build(chunk_id));

                let chunk = self.chunks.last_mut().unwrap();

                debug!(self.logger, "allocated chunk"; "chunk_id" => chunk_id.2);

                (chunk_index, chunk)
            }
        }
    }
}
