use crate::*;
use downcast_rs::{impl_downcast, Downcast};
use std::any::TypeId;
use std::boxed::FnBox;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::mem::size_of;
use std::sync::Arc;

impl_downcast!(ComponentStorage);
trait ComponentStorage: Downcast + Debug {
    fn remove(&mut self, id: ComponentID);
    fn len(&self) -> usize;
}

#[derive(Debug)]
struct UnsafeVec<T>(UnsafeCell<Vec<T>>);

impl<T: Debug> UnsafeVec<T> {
    fn with_capacity(capacity: usize) -> Self {
        UnsafeVec(UnsafeCell::new(Vec::<T>::with_capacity(capacity)))
    }

    unsafe fn inner(&self) -> &Vec<T> {
        &(*self.0.get())
    }

    unsafe fn inner_mut(&self) -> &mut Vec<T> {
        &mut (*self.0.get())
    }
}

impl<T: Debug + 'static> ComponentStorage for UnsafeVec<T> {
    fn remove(&mut self, id: ComponentID) {
        unsafe {
            self.inner_mut().swap_remove(id as usize);
        }
    }

    fn len(&self) -> usize {
        unsafe { self.inner_mut().len() }
    }
}

impl_downcast!(SharedComponentStorage);
trait SharedComponentStorage: Downcast + Debug {}

#[derive(Debug)]
struct SharedComponentStore<T>(UnsafeCell<T>);

impl<T: SharedComponent> SharedComponentStorage for SharedComponentStore<T> {}

#[derive(Debug)]
pub struct Chunk {
    capacity: usize,
    entities: UnsafeVec<Entity>,
    components: HashMap<TypeId, Box<dyn ComponentStorage>>,
    shared: HashMap<TypeId, Arc<dyn SharedComponentStorage>>,
}

impl Chunk {
    pub fn len(&self) -> usize {
        unsafe { self.entities.inner().len() }
    }

    pub fn is_full(&self) -> bool {
        self.len() == self.capacity
    }

    pub unsafe fn entities(&self) -> &Vec<Entity> {
        self.entities.inner()
    }

    pub unsafe fn entities_mut(&self) -> &mut Vec<Entity> {
        self.entities.inner_mut()
    }

    pub unsafe fn component_ids<'a>(
        &'a self,
    ) -> impl Iterator<Item = (Entity, ComponentID)> + ExactSizeIterator + 'a {
        self.entities
            .inner()
            .iter()
            .enumerate()
            .map(|(i, e)| (*e, i as ComponentID))
    }

    pub unsafe fn components<T: Component>(&self) -> Option<&Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<UnsafeVec<T>>())
            .map(|c| c.inner())
    }

    pub unsafe fn components_mut<T: Component>(&self) -> Option<&mut Vec<T>> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|c| c.downcast_ref::<UnsafeVec<T>>())
            .map(|c| c.inner_mut())
    }

    pub unsafe fn shared_component<T: SharedComponent>(&self) -> Option<&T> {
        self.shared
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<SharedComponentStore<T>>())
            .map(|s| &*s.0.get())
    }

    pub unsafe fn remove(&mut self, id: ComponentID) -> Option<Entity> {
        let index = id as usize;
        self.entities.inner_mut().swap_remove(index);
        for storage in self.components.values_mut() {
            storage.remove(id);
        }

        if self.entities.len() > index {
            Some(*self.entities.inner().get(index).unwrap())
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
}

pub struct ChunkBuilder {
    components: Vec<(
        TypeId,
        usize,
        Box<dyn FnBox(usize) -> Box<dyn ComponentStorage>>,
    )>,
    shared: HashMap<TypeId, Arc<dyn SharedComponentStorage>>,
}

impl ChunkBuilder {
    const MAX_SIZE: usize = 16 * 1024;

    pub fn new() -> ChunkBuilder {
        ChunkBuilder {
            components: Vec::new(),
            shared: HashMap::new(),
        }
    }

    pub fn register_component<T: Component>(&mut self) {
        let constructor = |capacity| {
            Box::new(UnsafeVec::<T>::with_capacity(capacity)) as Box<dyn ComponentStorage>
        };
        self.components
            .push((TypeId::of::<T>(), size_of::<T>(), Box::new(constructor)));
    }

    pub fn register_shared<T: SharedComponent>(&mut self, data: T) {
        self.shared.insert(
            TypeId::of::<T>(),
            Arc::new(SharedComponentStore(UnsafeCell::new(data)))
                as Arc<dyn SharedComponentStorage>,
        );
    }

    pub fn build(self) -> Chunk {
        let size_bytes = *self
            .components
            .iter()
            .map(|(_, size, _)| size)
            .max()
            .unwrap_or(&ChunkBuilder::MAX_SIZE);
        let capacity = std::cmp::max(1, ChunkBuilder::MAX_SIZE / size_bytes);
        Chunk {
            capacity: capacity,
            entities: UnsafeVec::with_capacity(capacity),
            components: self
                .components
                .into_iter()
                .map(|(id, _, con)| (id, con(capacity)))
                .collect(),
            shared: self.shared,
        }
    }
}

#[derive(Debug)]
pub struct Archetype {
    pub components: HashSet<TypeId>,
    pub shared: HashSet<TypeId>,
    pub chunks: Vec<Chunk>,
}

impl Archetype {
    pub fn new(components: HashSet<TypeId>, shared: HashSet<TypeId>) -> Archetype {
        Archetype {
            components: components,
            shared: shared,
            chunks: Vec::new(),
        }
    }

    pub fn chunk(&self, id: ChunkID) -> Option<&Chunk> {
        self.chunks.get(id as usize)
    }

    pub fn chunk_mut(&mut self, id: ChunkID) -> Option<&mut Chunk> {
        self.chunks.get_mut(id as usize)
    }

    pub fn has_component<T: Component>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    pub fn has_shared<T: SharedComponent>(&self) -> bool {
        self.shared.contains(&TypeId::of::<T>())
    }

    pub fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.iter()
    }

    pub fn chunks_with_space_mut(&mut self) -> impl Iterator<Item = &mut Chunk> {
        self.chunks.iter_mut().filter(|c| !c.is_full())
    }
}
