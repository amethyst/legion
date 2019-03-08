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
    pub fn id(&self) -> ChunkId {
        self.id
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_full(&self) -> bool {
        self.len() == self.capacity
    }

    pub unsafe fn entities(&self) -> &[Entity] {
        self.entities.data()
    }

    pub unsafe fn entities_unchecked(&self) -> &mut Vec<Entity> {
        self.entities.data_mut()
    }

    pub unsafe fn entity_data_unchecked<T: EntityData>(&self) -> Option<&Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data())
    }

    pub unsafe fn entity_data_mut_unchecked<T: EntityData>(&self) -> Option<&mut Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.data_mut())
    }

    pub fn entity_data<'a, T: EntityData>(&'a self) -> Option<BorrowedSlice<'a, T>> {
        match unsafe { self.entity_data_unchecked() } {
            Some(data) => {
                let borrow = self.borrow::<T>();
                Some(BorrowedSlice::new(data, borrow))
            }
            None => None,
        }
    }

    pub fn entity_data_mut<'a, T: EntityData>(&'a self) -> Option<BorrowedMutSlice<'a, T>> {
        match unsafe { self.entity_data_mut_unchecked() } {
            Some(data) => {
                let borrow = self.borrow_mut::<T>();
                Some(BorrowedMutSlice::new(data, borrow))
            }
            None => None,
        }
    }

    pub fn entity_data_version<T: EntityData>(&self) -> Option<usize> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<StorageVec<T>>())
            .map(|c| c.version())
    }

    pub unsafe fn shared_component<T: SharedData>(&self) -> Option<&T> {
        self.shared
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<SharedComponentStore<T>>())
            .map(|s| &*s.0.get())
    }

    pub unsafe fn remove(&mut self, id: ComponentIndex) -> Option<Entity> {
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

    pub fn new() -> ChunkBuilder {
        ChunkBuilder {
            components: Vec::new(),
            shared: FnvHashMap::default(),
        }
    }

    pub fn register_component<T: EntityData>(&mut self) {
        let constructor = |capacity| {
            Box::new(StorageVec::<T>::with_capacity(capacity)) as Box<dyn ComponentStorage>
        };
        self.components
            .push((TypeId::of::<T>(), size_of::<T>(), Box::new(constructor)));
    }

    pub fn register_shared<T: SharedData>(&mut self, data: T) {
        self.shared.insert(
            TypeId::of::<T>(),
            Arc::new(SharedComponentStore(UnsafeCell::new(data)))
                as Arc<dyn SharedComponentStorage>,
        );
    }

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

#[derive(Debug)]
pub struct Archetype {
    id: ArchetypeId,
    logger: slog::Logger,
    next_chunk_id: u16,
    pub components: FnvHashSet<TypeId>,
    pub shared: FnvHashSet<TypeId>,
    pub chunks: Vec<Chunk>,
}

impl Archetype {
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

    pub fn chunk(&self, id: ChunkIndex) -> Option<&Chunk> {
        self.chunks.get(id as usize)
    }

    pub fn chunk_mut(&mut self, id: ChunkIndex) -> Option<&mut Chunk> {
        self.chunks.get_mut(id as usize)
    }

    pub fn has_component<T: EntityData>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    pub fn has_shared<T: SharedData>(&self) -> bool {
        self.shared.contains(&TypeId::of::<T>())
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn get_or_create_chunk<'a, 'b, 'c, S: SharedDataSet, C: ComponentSource>(
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
