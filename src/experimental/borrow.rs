use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicIsize, Ordering};

pub struct AtomicRefCell<T> {
    value: UnsafeCell<T>,
    borrow_state: AtomicIsize,
}

impl<T> AtomicRefCell<T> {
    pub fn new(value: T) -> Self {
        AtomicRefCell {
            value: UnsafeCell::from(value),
            borrow_state: AtomicIsize::from(0),
        }
    }

    #[inline(always)]
    pub fn get<'a>(&'a self) -> Ref<'a, Shared, T> { self.try_get().unwrap() }

    #[cfg(debug_assertions)]
    pub fn try_get<'a>(&'a self) -> Result<Ref<'a, Shared<'a>, T>, &'static str> {
        loop {
            let read = self.borrow_state.load(Ordering::SeqCst);
            if read < 0 {
                return Err("resource already borrowed as mutable");
            }

            if self
                .borrow_state
                .compare_and_swap(read, read + 1, Ordering::SeqCst)
                == read
            {
                break;
            }
        }

        Ok(Ref::new(Shared::new(&self.borrow_state), unsafe {
            &*self.value.get()
        }))
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn try_get<'a>(&'a self) -> Result<Ref<'a, Shared<'a>, T>, &'static str> {
        Ok(Ref::new(Shared::new(&self.borrow_state), unsafe {
            &*self.value.get()
        }))
    }

    #[inline(always)]
    pub fn get_mut<'a>(&'a self) -> RefMut<'a, Exclusive, T> { self.try_get_mut().unwrap() }

    #[cfg(debug_assertions)]
    pub fn try_get_mut<'a>(&'a self) -> Result<RefMut<'a, Exclusive<'a>, T>, &'static str> {
        let borrowed = self.borrow_state.compare_and_swap(0, -1, Ordering::SeqCst);
        match borrowed {
            0 => Ok(RefMut::new(Exclusive::new(&self.borrow_state), unsafe {
                &mut *self.value.get()
            })),
            x if x < 0 => Err("resource already borrowed as mutable"),
            _ => Err("resource already borrowed as immutable"),
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn try_get_mut<'a>(&'a self) -> Result<RefMut<'a, Exclusive<'a>, T>, &'static str> {
        Ok(RefMut::new(Exclusive::new(&self.borrow_state), unsafe {
            &mut *self.value.get()
        }))
    }
}

pub trait UnsafeClone {
    unsafe fn clone(&self) -> Self;
}

impl<A: UnsafeClone, B: UnsafeClone> UnsafeClone for (A, B) {
    unsafe fn clone(&self) -> Self { (self.0.clone(), self.1.clone()) }
}

#[derive(Debug)]
pub struct Shared<'a> {
    #[cfg(debug_assertions)]
    state: &'a AtomicIsize,
    #[cfg(not(debug_assertions))]
    state: PhantomData<&'a ()>,
}

impl<'a> Shared<'a> {
    #[cfg(debug_assertions)]
    fn new(state: &'a AtomicIsize) -> Self { Self { state } }
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn new(_: &'a AtomicIsize) -> Self { Self { state: PhantomData } }
}

#[cfg(debug_assertions)]
impl<'a> Drop for Shared<'a> {
    fn drop(&mut self) { self.state.fetch_sub(1, Ordering::SeqCst); }
}

impl<'a> Clone for Shared<'a> {
    fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        self.state.fetch_add(1, Ordering::SeqCst);
        Shared { state: self.state }
    }
}

impl<'a> UnsafeClone for Shared<'a> {
    unsafe fn clone(&self) -> Self { std::clone::Clone::clone(&self) }
}

#[derive(Debug)]
pub struct Exclusive<'a> {
    #[cfg(debug_assertions)]
    state: &'a AtomicIsize,
    #[cfg(not(debug_assertions))]
    state: PhantomData<&'a ()>,
}

impl<'a> Exclusive<'a> {
    #[cfg(debug_assertions)]
    fn new(state: &'a AtomicIsize) -> Self { Self { state } }
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn new(_: &'a AtomicIsize) -> Self { Self { state: PhantomData } }
}

#[cfg(debug_assertions)]
impl<'a> Drop for Exclusive<'a> {
    fn drop(&mut self) { self.state.fetch_add(1, Ordering::SeqCst); }
}

impl<'a> UnsafeClone for Exclusive<'a> {
    unsafe fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        self.state.fetch_sub(1, Ordering::SeqCst);
        Exclusive { state: self.state }
    }
}

#[derive(Debug)]
pub struct Ref<'a, State: 'a, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: &'a T,
}

impl<'a, State: 'a + Clone, T: 'a> Clone for Ref<'a, State, T> {
    fn clone(&self) -> Self { Ref::new(self.borrow.clone(), self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Ref<'a, State, T> {
    pub fn new(borrow: State, value: &'a T) -> Self { Self { borrow, value } }

    pub fn map_into<K: 'a, F: FnMut(&'a T) -> K>(self, mut f: F) -> RefMap<'a, State, K> {
        RefMap::new(self.borrow, f(&self.value))
    }

    pub fn map<K: 'a, F: FnMut(&T) -> &K>(&self, mut f: F) -> Ref<'a, State, K> {
        Ref::new(self.borrow.clone(), f(&self.value))
    }

    pub unsafe fn deconstruct(self) -> (State, &'a T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Deref for Ref<'a, State, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.value }
}

impl<'a, State: 'a + Clone, T: 'a> AsRef<T> for Ref<'a, State, T> {
    fn as_ref(&self) -> &T { self.value }
}

impl<'a, State: 'a + Clone, T: 'a> std::borrow::Borrow<T> for Ref<'a, State, T> {
    fn borrow(&self) -> &T { self.value }
}

#[derive(Debug)]
pub struct RefMut<'a, State: 'a + UnsafeClone, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: &'a mut T,
}

impl<'a, State: 'a + UnsafeClone, T: 'a> RefMut<'a, State, T> {
    pub fn new(borrow: State, value: &'a mut T) -> Self { Self { borrow, value } }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, State, K> {
        RefMapMut::new(self.borrow, f(&mut self.value))
    }

    pub unsafe fn deconstruct(self) -> (State, &'a mut T) { (self.borrow, self.value) }

    pub fn split<First, Rest, F: Fn(&'a mut T) -> (&'a mut First, &'a mut Rest)>(
        self,
        f: F,
    ) -> (RefMut<'a, State, First>, RefMut<'a, State, Rest>) {
        let (first, rest) = f(self.value);
        (
            RefMut::new(unsafe { self.borrow.clone() }, first),
            RefMut::new(self.borrow, rest),
        )
    }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> Deref for RefMut<'a, State, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> DerefMut for RefMut<'a, State, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> AsRef<T> for RefMut<'a, State, T> {
    fn as_ref(&self) -> &T { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> std::borrow::Borrow<T> for RefMut<'a, State, T> {
    fn borrow(&self) -> &T { self.value }
}

#[derive(Debug)]
pub struct RefMap<'a, State: 'a + Clone, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: T,
    _phantom: PhantomData<&'a State>,
}

impl<'a, State: 'a + Clone, T: 'a> RefMap<'a, State, T> {
    pub fn new(borrow: State, value: T) -> Self {
        Self {
            borrow,
            value,
            _phantom: PhantomData,
        }
    }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMap<'a, State, K> {
        RefMap::new(self.borrow, f(&mut self.value))
    }

    pub unsafe fn deconstruct(self) -> (State, T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Deref for RefMap<'a, State, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { &self.value }
}

impl<'a, State: 'a + Clone, T: 'a> AsRef<T> for RefMap<'a, State, T> {
    fn as_ref(&self) -> &T { &self.value }
}

impl<'a, State: 'a + Clone, T: 'a> std::borrow::Borrow<T> for RefMap<'a, State, T> {
    fn borrow(&self) -> &T { &self.value }
}

impl<'a, State: 'a + Clone, I: 'a + Iterator> Iterator for RefMap<'a, State, I> {
    type Item = RefMap<'a, State, I::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.value.next() {
            Some(RefMap::new(self.borrow.clone(), item))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) { self.value.size_hint() }
}

impl<'a, State: 'a + Clone, I: 'a + ExactSizeIterator> ExactSizeIterator for RefMap<'a, State, I> {}

#[derive(Debug)]
pub struct RefMapMut<'a, State: 'a + UnsafeClone, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: T,
    _phantom: PhantomData<&'a State>,
}

impl<'a, State: 'a + UnsafeClone, T: 'a> RefMapMut<'a, State, T> {
    pub fn new(borrow: State, value: T) -> Self {
        Self {
            borrow,
            value,
            _phantom: PhantomData,
        }
    }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, State, K> {
        RefMapMut {
            value: f(&mut self.value),
            borrow: self.borrow,
            _phantom: PhantomData,
        }
    }

    pub unsafe fn deconstruct(self) -> (State, T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> Deref for RefMapMut<'a, State, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { &self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> DerefMut for RefMapMut<'a, State, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> AsRef<T> for RefMapMut<'a, State, T> {
    fn as_ref(&self) -> &T { &self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> std::borrow::Borrow<T> for RefMapMut<'a, State, T> {
    fn borrow(&self) -> &T { &self.value }
}

impl<'a, State: 'a + UnsafeClone, I: 'a + Iterator> Iterator for RefMapMut<'a, State, I> {
    type Item = RefMapMut<'a, State, I::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.value.next() {
            Some(RefMapMut::new(unsafe { self.borrow.clone() }, item))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) { self.value.size_hint() }
}

impl<'a, State: 'a + UnsafeClone, I: 'a + ExactSizeIterator> ExactSizeIterator
    for RefMapMut<'a, State, I>
{
}
