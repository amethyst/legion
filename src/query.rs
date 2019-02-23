use std::slice::IterMut;
use std::iter::Zip;
use std::iter::Take;
use std::iter::Repeat;
use std::slice::Iter;
use std::marker::PhantomData;

use crate::*;

pub trait View<'a>: Sized + 'static {
    type Iter: Iterator + 'a;
    type Filter: ArchetypeFilter;
    
    fn fetch(chunk: &'a Chunk) -> Self::Iter;
    fn filter() -> Self::Filter;
}

trait Queryable<'a, World>: View<'a> {
    fn query(world: World) -> Query<'a, Self, <Self as View<'a>>::Filter, Passthrough>;
}

impl<'a, T: View<'a>> Queryable<'a, &'a mut World> for T {     
    fn query(world: &'a mut World) -> Query<'a, Self, Self::Filter, Passthrough> {        
        Query {
            world: world,
            view: PhantomData,
            arch_filter: Self::filter(),
            chunk_filter: Passthrough
        }
    }
}

trait ReadOnly {}

impl<'a, T: View<'a> + ReadOnly> Queryable<'a, &'a World> for T {
    fn query(world: &'a World) -> Query<'a, Self, Self::Filter, Passthrough> {
        Query {
            world,
            view: PhantomData,
            arch_filter: Self::filter(),
            chunk_filter: Passthrough
        }
    }
}

#[derive(Debug)]
pub struct Read<T: Component>(PhantomData<T>);

impl<T: Component> ReadOnly for Read<T> {}

impl<'a, T: Component> View<'a> for Read<T> {
    type Iter = Iter<'a, T>;
    type Filter = EntityDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        unsafe { chunk.components().unwrap().iter() }
    }

    fn filter() -> Self::Filter {
        EntityDataFilter::new()
    }
}

#[derive(Debug)]
pub struct Write<T: Component>(PhantomData<T>);

impl<'a, T: Component> View<'a> for Write<T> {
    type Iter = IterMut<'a, T>;
    type Filter = EntityDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        unsafe { chunk.components_mut().unwrap().iter_mut() }
    }

    fn filter() -> Self::Filter {
        EntityDataFilter::new()
    }
}

#[derive(Debug)]
pub struct Shared<T: SharedComponent>(PhantomData<T>);

impl<T: SharedComponent> ReadOnly for Shared<T> {}

impl<'a, T: SharedComponent> View<'a> for Shared<T> {
    type Iter = Take<Repeat<&'a T>>;
    type Filter = SharedDataFilter<T>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        unsafe {
            let data: &T = chunk.shared_component().unwrap();
            std::iter::repeat(data).take(chunk.len())
        }
    }

    fn filter() -> Self::Filter {
        SharedDataFilter::new()
    }
}

impl<'a, T1: View<'a>, T2: View<'a>> View<'a> for (T1, T2) {
    type Iter = Zip<T1::Iter, T2::Iter>;
    type Filter = And<T1::Filter, T2::Filter>;

    fn fetch(chunk: &'a Chunk) -> Self::Iter {
        T1::fetch(chunk).zip(T2::fetch(chunk))
    }

    fn filter() -> Self::Filter {
        And { a: T1::filter(), b: T2::filter() }
    }
}

impl<T1: ReadOnly, T2: ReadOnly> ReadOnly for (T1, T2) { }

pub trait ArchetypeFilter {
    fn filter(&self, archetype: &Archetype) -> bool;
}

pub trait ChunkFilter {
    fn filter(&self, chunk: &Chunk) -> bool;
}

#[derive(Debug)]
pub struct Passthrough;

impl ArchetypeFilter for Passthrough {
    #[inline]
    fn filter(&self, _: &Archetype) -> bool {
        true
    }
}

impl ChunkFilter for Passthrough {
    #[inline]
    fn filter(&self, _: &Chunk) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct Not<F> {
    filter: F
}

impl<F: ArchetypeFilter> ArchetypeFilter for Not<F> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        !self.filter.filter(archetype)
    }
}

impl<F: ChunkFilter> ChunkFilter for Not<F> {
    #[inline]
    fn filter(&self, chunk: &Chunk) -> bool {
        !self.filter.filter(chunk)
    }
}

#[derive(Debug)]
pub struct And<A, B> {
    a: A,
    b: B
}

impl<A: ArchetypeFilter, B: ArchetypeFilter> ArchetypeFilter for And<A, B> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        self.a.filter(archetype) && self.b.filter(archetype)
    }
}

impl<A: ChunkFilter, B: ChunkFilter> ChunkFilter for And<A, B> {
    #[inline]
    fn filter(&self, chunk: &Chunk) -> bool {
        self.a.filter(chunk) && self.b.filter(chunk)
    }
}

#[derive(Debug)]
pub struct EntityDataFilter<T>(PhantomData<T>);

impl<T: Component> EntityDataFilter<T> {
    fn new() -> Self {
        EntityDataFilter(PhantomData)
    }
}

impl<T: Component> ArchetypeFilter for EntityDataFilter<T> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        archetype.has_component::<T>()
    }
}

#[derive(Debug)]
pub struct SharedDataFilter<T>(PhantomData<T>);

impl<T: SharedComponent> SharedDataFilter<T> {
    fn new() -> Self {
        SharedDataFilter(PhantomData)
    }
}

impl<T: SharedComponent> ArchetypeFilter for SharedDataFilter<T> {
    #[inline]
    fn filter(&self, archetype: &Archetype) -> bool {
        archetype.has_shared::<T>()
    }
}

#[derive(Debug)]
pub struct SharedDataValueFilter<'a, T> {
    value: &'a T
}

impl<'a, T: SharedComponent> SharedDataValueFilter<'a, T> {
    fn new(value: &'a T) -> Self {
        SharedDataValueFilter {
            value
        }
    }
}

impl<'a, T: SharedComponent> ChunkFilter for SharedDataValueFilter<'a, T> {
    #[inline]
    fn filter(&self, chunk: &Chunk) -> bool {
        unsafe { chunk.shared_component::<T>() }.map_or(false, |s| s == self.value)
    }
}

#[derive(Debug)]
pub struct Query<'a, V: View<'a>, A: ArchetypeFilter, C: ChunkFilter> {
    world: &'a World,
    view: PhantomData<V>,
    arch_filter: A,
    chunk_filter: C
}

impl<'a, V: View<'a>, A: ArchetypeFilter, C: ChunkFilter> Query<'a, V, A, C> 
where A: 'a,
      C: 'a 
{
    pub fn with_entity_data<T: Component>(self) -> Query<'a, V, And<A, EntityDataFilter<T>>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And { a: self.arch_filter, b: EntityDataFilter::new() },
            chunk_filter: self.chunk_filter
        } 
    }

    pub fn without_entity_data<T: Component>(self) -> Query<'a, V, And<A, Not<EntityDataFilter<T>>>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And { a: self.arch_filter, b: Not { filter: EntityDataFilter::new() } },
            chunk_filter: self.chunk_filter
        } 
    }

    pub fn with_shared_data<T: SharedComponent>(self) -> Query<'a, V, And<A, SharedDataFilter<T>>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And { a: self.arch_filter, b: SharedDataFilter::new() },
            chunk_filter: self.chunk_filter
        } 
    }

    pub fn without_shared_data<T: SharedComponent>(self) -> Query<'a, V, And<A, Not<SharedDataFilter<T>>>, C> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: And { a: self.arch_filter, b: Not { filter: SharedDataFilter::new() } },
            chunk_filter: self.chunk_filter
        } 
    }

    pub fn with_shared_data_value<'b, T: SharedComponent>(self, value: &'b T) -> Query<'a, V, A, And<C, SharedDataValueFilter<'b, T>>> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: self.arch_filter,
            chunk_filter: And { a: self.chunk_filter, b: SharedDataValueFilter::new(value) }
        } 
    }

    pub fn without_shared_data_value<'b, T: SharedComponent>(self, value: &'b T) -> Query<'a, V, A, And<C, Not<SharedDataValueFilter<'b, T>>>> {
        Query {
            world: self.world,
            view: self.view,
            arch_filter: self.arch_filter,
            chunk_filter: And { a: self.chunk_filter, b: Not { filter: SharedDataValueFilter::new(value) } }
        } 
    }

    pub fn into_chunks(self) -> impl Iterator<Item = ChunkView<'a, V>> {
        let world = self.world;
        let arch = self.arch_filter;
        let chunk = self.chunk_filter;
        world.archetypes.iter()
            .filter(move |a| arch.filter(a))
            .flat_map(|a| a.chunks())
            .filter(move |c| chunk.filter(c))
            .map(|c| ChunkView { chunk: c, view: PhantomData })
    }

    pub fn into_data(self) -> impl Iterator<Item = <<V as View<'a>>::Iter as Iterator>::Item> {
        self.into_chunks().flat_map(|mut c| c.data())
    }

    pub fn into_data_with_entities(self) -> impl Iterator<Item = (Entity,  <<V as View<'a>>::Iter as Iterator>::Item)> {
        self.into_chunks().flat_map(|mut c| c.data_with_entities())
    }
}

#[derive(Debug)]
pub struct ChunkView<'a, V: View<'a>> {
    chunk: &'a Chunk,
    view: PhantomData<V>
}

impl<'a, V: View<'a>> ChunkView<'a, V>
{
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        unsafe { self.chunk.entities().iter() }
    }

    pub fn data(&mut self) -> V::Iter {
        V::fetch(self.chunk)
    }

    pub fn data_with_entities(&mut self) -> impl Iterator<Item = (Entity, <<V as View<'a>>::Iter as Iterator>::Item)> + 'a {
        unsafe {
            self.chunk.entities().iter()
                .map(|e| *e)
                .zip(V::fetch(self.chunk))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use crate::query::*;
    use std::iter::FromIterator;

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

    fn create_test_world() -> (World, HashMap<Entity, (Option<Pos>, Option<Rot>, Option<Vel>, Option<Model>, Option<Static>)>) {
        let universe = Universe::new(None);
        let mut world = universe.create_world();
        let mut expected: HashMap<Entity, (Option<Pos>, Option<Rot>, Option<Vel>, Option<Model>, Option<Static>)> = HashMap::new();

        // pos, rot
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
        for (i, e) in world.insert_from((), data.clone()).iter().enumerate() {
            let (pos, rot) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), None, None, None));
        }

        // model(1) | pos, rot
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
        for (i, e) in world.insert_from((Model(1),), data.clone()).iter().enumerate() {
            let (pos, rot) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(1)), None));
        }

        // model(2) | pos, rot
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
        for (i, e) in world.insert_from((Model(2),), data.clone()).iter().enumerate() {
            let (pos, rot) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(2)), None));
        }

        // static | pos, rot
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
        for (i, e) in world.insert_from((Static,), data.clone()).iter().enumerate() {
            let (pos, rot) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), None, None, Some(Static)));
        }

        // static, model(1) | pos, rot
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {*x += 1.; Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.))) }).take(1000));
        for (i, e) in world.insert_from((Static, Model(1)), data.clone()).iter().enumerate() {
            let (pos, rot) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), None, Some(Model(1)), Some(Static)));
        }

        // pos, rot, vel
        let data = Vec::from_iter(std::iter::unfold(0f32, |x| {
            *x += 1.;
            Some((Pos(*x + 1., *x + 1., *x + 2.), Rot(*x + 3., *x + 4., *x + 5.), Vel(*x + 6., *x + 7., *x + 8.))) 
        }).take(1000));
        for (i, e) in world.insert_from((), data.clone()).iter().enumerate() {
            let (pos, rot, vel) = data.get(i).unwrap();
            expected.insert(*e, (Some(*pos), Some(*rot), Some(*vel), None, None));
        }

        (world, expected)
    }

    #[test]
    fn query_read_entity_data() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = Read::<Pos>::query(&world);

        let mut count = 0;
        for (entity, pos) in query.into_data_with_entities() {
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            count += 1;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_read_entity_data_tuple() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = <(Read<Pos>, Read<Rot>)>::query(&world);

        let mut count = 0;
        for (entity, (pos, rot)) in query.into_data_with_entities() {
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            assert_eq!(&expected.get(&entity).unwrap().1, rot);
            count += 1;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_write_entity_data() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = Write::<Pos>::query(&mut world);

        let mut count = 0;
        for (entity, pos) in query.into_data_with_entities() {            
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            count += 1;

            pos.0 = 0.0;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_write_entity_data_tuple() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = <(Write<Pos>, Write<Rot>)>::query(&mut world);

        let mut count = 0;
        for (entity, (pos, rot)) in query.into_data_with_entities() {            
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            assert_eq!(&expected.get(&entity).unwrap().1, rot);
            count += 1;

            pos.0 = 0.0;
            rot.0 = 0.0;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_mixed_entity_data_tuple() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = <(Read<Pos>, Write<Rot>)>::query(&mut world);

        let mut count = 0;
        for (entity, (pos, rot)) in query.into_data_with_entities() {            
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            assert_eq!(&expected.get(&entity).unwrap().1, rot);
            count += 1;

            rot.0 = 0.0;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_partial_match() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        let mut expected = HashMap::<Entity, (Pos, Rot)>::new();

        for (i, e) in world.insert_from(shared, components.clone()).iter().enumerate() {
            if let Some((pos, rot)) = components.get(i) {
                expected.insert(*e, (*pos, *rot));
            }
        }

        let query = <(Read<Pos>, Write<Rot>)>::query(&mut world);

        let mut count = 0;
        for (entity, (pos, rot)) in query.into_data_with_entities() {            
            assert_eq!(&expected.get(&entity).unwrap().0, pos);
            assert_eq!(&expected.get(&entity).unwrap().1, rot);
            count += 1;

            rot.0 = 0.0;
        }

        assert_eq!(components.len(), count);
    }

    #[test]
    fn query_read_shared_data() {
        let universe = Universe::new(None);
        let mut world = universe.create_world();

        let shared = (Static, Model(5));
        let components = vec![
            (Pos(1., 2., 3.), Rot(0.1, 0.2, 0.3)),
            (Pos(4., 5., 6.), Rot(0.4, 0.5, 0.6))
        ];

        world.insert_from(shared, components.clone());

        let query = Shared::<Static>::query(&world);

        let mut count = 0;
        for marker in query.into_data() {
            assert_eq!(&Static, marker);
            count += 1;
        }

        assert_eq!(components.len(), count);
    }
}