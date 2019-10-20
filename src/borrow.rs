//! Atomic runtime borrow checking module.  
//! These types implement something akin to `RefCell`, but are atomically handled allowing them to
//! cross thread boundaries.
use std::cell::UnsafeCell;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::atomic::AtomicIsize;

/// A `RefCell` implementation which is thread safe. This type performs all the standard runtime
/// borrow checking which would be familiar from using `RefCell`.
///
/// `UnsafeCell` is used in this type, but borrow checking is performed using atomic values,
/// garunteeing safe access across threads.
///
/// # Safety
/// Runtime borrow checking is only conducted in builds with `debug_assertions` enabled. Release
/// builds assume proper resource access and will cause undefined behavior with improper use.
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

    /// Retrieve an immutable `Ref` wrapped reference of `&T`.
    ///
    /// # Panics
    ///
    /// This method panics if this value is already mutably borrowed.
    ///
    /// # Safety
    /// Runtime borrow checking is only conducted in builds with `debug_assertions` enabled. Release
    /// builds assume proper resource access and will cause undefined behavior with improper use.
    #[inline(always)]
    pub fn get<'a>(&'a self) -> Ref<'a, Shared, T> { self.try_get().unwrap() }

    /// Unwrap the value from the RefCell and kill it, returning the value.
    pub fn into_inner(self) -> T { self.value.into_inner() }

    /// Retrieve an immutable `Ref` wrapped reference of `&T`. This is the safe version of `get`
    /// providing an error result on failure.
    ///
    /// # Returns
    ///
    /// `Some(T)` if the value can be retrieved.
    /// `Err` if the value is already mutably borrowed.
    #[cfg(debug_assertions)]
    pub fn try_get<'a>(&'a self) -> Result<Ref<'a, Shared<'a>, T>, &'static str> {
        loop {
            let read = self.borrow_state.load(std::sync::atomic::Ordering::SeqCst);
            if read < 0 {
                return Err("resource already borrowed as mutable");
            }

            if self.borrow_state.compare_and_swap(
                read,
                read + 1,
                std::sync::atomic::Ordering::SeqCst,
            ) == read
            {
                break;
            }
        }

        Ok(Ref::new(Shared::new(&self.borrow_state), unsafe {
            &*self.value.get()
        }))
    }

    /// Retrieve an immutable `Ref` wrapped reference of `&T`. This is the safe version of `get`
    /// providing an error result on failure.
    ///
    /// # Returns
    ///
    /// `Some(T)` if the value can be retrieved.
    /// `Err` if the value is already mutably borrowed.
    ///
    /// # Safety
    ///
    /// This release version of this function does not perform runtime borrow checking and will
    /// cause undefined behavior if borrow rules are violated. This means they should be enforced
    /// on the use of this type.
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn try_get<'a>(&'a self) -> Result<Ref<'a, Shared<'a>, T>, &'static str> {
        Ok(Ref::new(Shared::new(&self.borrow_state), unsafe {
            &*self.value.get()
        }))
    }

    /// Retrieve an mutable `RefMut` wrapped reference of `&mut T`.
    ///
    /// # Panics
    ///
    /// This method panics if this value is already mutably borrowed.
    ///
    /// # Safety
    /// Runtime borrow checking is only conducted in builds with `debug_assertions` enabled. Release
    /// builds assume proper resource access and will cause undefined behavior with improper use.
    #[inline(always)]
    pub fn get_mut<'a>(&'a self) -> RefMut<'a, Exclusive, T> { self.try_get_mut().unwrap() }

    /// Retrieve a mutable `RefMut` wrapped reference of `&mut T`. This is the safe version of
    /// `get_mut` providing an error result on failure.
    ///
    /// # Returns
    ///
    /// `Some(T)` if the value can be retrieved.
    /// `Err` if the value is already mutably borrowed.
    ///
    /// # Safety
    ///
    /// This release version of this function does not perform runtime borrow checking and will
    /// cause undefined behavior if borrow rules are violated. This means they should be enforced
    /// on the use of this type.
    #[cfg(debug_assertions)]
    pub fn try_get_mut<'a>(&'a self) -> Result<RefMut<'a, Exclusive<'a>, T>, &'static str> {
        let borrowed =
            self.borrow_state
                .compare_and_swap(0, -1, std::sync::atomic::Ordering::SeqCst);
        match borrowed {
            0 => Ok(RefMut::new(Exclusive::new(&self.borrow_state), unsafe {
                &mut *self.value.get()
            })),
            x if x < 0 => Err("resource already borrowed as mutable"),
            _ => Err("resource already borrowed as immutable"),
        }
    }

    /// Retrieve a mutable `RefMut` wrapped reference of `&mut T`. This is the safe version of
    /// `get_mut` providing an error result on failure.
    ///
    /// # Returns
    ///
    /// `Some(T)` if the value can be retrieved.
    /// `Err` if the value is already mutably borrowed.
    ///
    /// # Safety
    ///
    /// This release version of this function does not perform runtime borrow checking and will
    /// cause undefined behavior if borrow rules are violated. This means they should be enforced
    /// on the use of this type.
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn try_get_mut<'a>(&'a self) -> Result<RefMut<'a, Exclusive<'a>, T>, &'static str> {
        Ok(RefMut::new(Exclusive::new(&self.borrow_state), unsafe {
            &mut *self.value.get()
        }))
    }
}

unsafe impl<T: Send> Send for AtomicRefCell<T> {}

unsafe impl<T: Sync> Sync for AtomicRefCell<T> {}

/// Type used for allowing unsafe cloning of internal types
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
    fn drop(&mut self) { self.state.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); }
}

impl<'a> Clone for Shared<'a> {
    #[inline(always)]
    fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        self.state.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    fn drop(&mut self) { self.state.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }
}

impl<'a> UnsafeClone for Exclusive<'a> {
    #[inline(always)]
    unsafe fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        self.state.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
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
    #[inline(always)]
    fn clone(&self) -> Self { Ref::new(self.borrow.clone(), self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Ref<'a, State, T> {
    #[inline(always)]
    pub fn new(borrow: State, value: &'a T) -> Self { Self { borrow, value } }

    #[inline(always)]
    pub fn map_into<K: 'a, F: FnMut(&'a T) -> K>(self, mut f: F) -> RefMap<'a, State, K> {
        RefMap::new(self.borrow, f(&self.value))
    }

    #[inline(always)]
    pub fn map<K: 'a, F: FnMut(&T) -> &K>(&self, mut f: F) -> Ref<'a, State, K> {
        Ref::new(self.borrow.clone(), f(&self.value))
    }

    #[inline(always)]
    pub unsafe fn deconstruct(self) -> (State, &'a T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Deref for Ref<'a, State, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target { self.value }
}

impl<'a, State: 'a + Clone, T: 'a> AsRef<T> for Ref<'a, State, T> {
    #[inline(always)]
    fn as_ref(&self) -> &T { self.value }
}

impl<'a, State: 'a + Clone, T: 'a> std::borrow::Borrow<T> for Ref<'a, State, T> {
    #[inline(always)]
    fn borrow(&self) -> &T { self.value }
}

impl<'a, State: 'a + Clone, T> PartialEq for Ref<'a, State, T>
where
    T: 'a + PartialEq,
{
    fn eq(&self, other: &Self) -> bool { self.value == other.value }
}
impl<'a, State: 'a + Clone, T> Eq for Ref<'a, State, T> where T: 'a + Eq {}

impl<'a, State: 'a + Clone, T> PartialOrd for Ref<'a, State, T>
where
    T: 'a + PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}
impl<'a, State: 'a + Clone, T> Ord for Ref<'a, State, T>
where
    T: 'a + Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.value.cmp(&other.value) }
}

impl<'a, State: 'a + Clone, T> Hash for Ref<'a, State, T>
where
    T: 'a + Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) { self.value.hash(state); }
}

#[derive(Debug)]
pub struct RefMut<'a, State: 'a + UnsafeClone, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: &'a mut T,
}

impl<'a, State: 'a + UnsafeClone, T: 'a> RefMut<'a, State, T> {
    #[inline(always)]
    pub fn new(borrow: State, value: &'a mut T) -> Self { Self { borrow, value } }

    #[inline(always)]
    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, State, K> {
        RefMapMut::new(self.borrow, f(&mut self.value))
    }

    #[inline(always)]
    pub unsafe fn deconstruct(self) -> (State, &'a mut T) { (self.borrow, self.value) }

    #[inline(always)]
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

    #[inline(always)]
    fn deref(&self) -> &Self::Target { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> DerefMut for RefMut<'a, State, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> AsRef<T> for RefMut<'a, State, T> {
    #[inline(always)]
    fn as_ref(&self) -> &T { self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> std::borrow::Borrow<T> for RefMut<'a, State, T> {
    #[inline(always)]
    fn borrow(&self) -> &T { self.value }
}

impl<'a, State: 'a + UnsafeClone, T> PartialEq for RefMut<'a, State, T>
where
    T: 'a + PartialEq,
{
    fn eq(&self, other: &Self) -> bool { self.value == other.value }
}
impl<'a, State: 'a + UnsafeClone, T> Eq for RefMut<'a, State, T> where T: 'a + Eq {}

impl<'a, State: 'a + UnsafeClone, T> PartialOrd for RefMut<'a, State, T>
where
    T: 'a + PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}
impl<'a, State: 'a + UnsafeClone, T> Ord for RefMut<'a, State, T>
where
    T: 'a + Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.value.cmp(&other.value) }
}

impl<'a, State: 'a + UnsafeClone, T> Hash for RefMut<'a, State, T>
where
    T: 'a + Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) { self.value.hash(state); }
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
    #[inline(always)]
    pub fn new(borrow: State, value: T) -> Self {
        Self {
            borrow,
            value,
            _phantom: PhantomData,
        }
    }

    #[inline(always)]
    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMap<'a, State, K> {
        RefMap::new(self.borrow, f(&mut self.value))
    }

    #[inline(always)]
    pub unsafe fn deconstruct(self) -> (State, T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + Clone, T: 'a> Deref for RefMap<'a, State, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target { &self.value }
}

impl<'a, State: 'a + Clone, T: 'a> AsRef<T> for RefMap<'a, State, T> {
    #[inline(always)]
    fn as_ref(&self) -> &T { &self.value }
}

impl<'a, State: 'a + Clone, T: 'a> std::borrow::Borrow<T> for RefMap<'a, State, T> {
    #[inline(always)]
    fn borrow(&self) -> &T { &self.value }
}

#[derive(Debug)]
pub struct RefMapMut<'a, State: 'a + UnsafeClone, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    value: T,
    _phantom: PhantomData<&'a State>,
}

impl<'a, State: 'a + UnsafeClone, T: 'a> RefMapMut<'a, State, T> {
    #[inline(always)]
    pub fn new(borrow: State, value: T) -> Self {
        Self {
            borrow,
            value,
            _phantom: PhantomData,
        }
    }

    #[inline(always)]
    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, State, K> {
        RefMapMut {
            value: f(&mut self.value),
            borrow: self.borrow,
            _phantom: PhantomData,
        }
    }

    #[inline(always)]
    pub unsafe fn deconstruct(self) -> (State, T) { (self.borrow, self.value) }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> Deref for RefMapMut<'a, State, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target { &self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> DerefMut for RefMapMut<'a, State, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> AsRef<T> for RefMapMut<'a, State, T> {
    #[inline(always)]
    fn as_ref(&self) -> &T { &self.value }
}

impl<'a, State: 'a + UnsafeClone, T: 'a> std::borrow::Borrow<T> for RefMapMut<'a, State, T> {
    #[inline(always)]
    fn borrow(&self) -> &T { &self.value }
}

#[derive(Debug)]
pub struct RefIter<'a, State: 'a + Clone, T: 'a, I: Iterator<Item = &'a T>> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    iter: I,
    _phantom: PhantomData<&'a State>,
}

impl<'a, State: 'a + Clone, T: 'a, I: Iterator<Item = &'a T>> RefIter<'a, State, T, I> {
    #[inline(always)]
    pub fn new(borrow: State, iter: I) -> Self {
        Self {
            borrow,
            iter,
            _phantom: PhantomData,
        }
    }
}

impl<'a, State: 'a + Clone, T: 'a, I: Iterator<Item = &'a T>> Iterator
    for RefIter<'a, State, T, I>
{
    type Item = Ref<'a, State, T>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.iter.next() {
            Some(Ref::new(self.borrow.clone(), item))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) { self.iter.size_hint() }
}

impl<'a, State: 'a + Clone, T: 'a, I: Iterator<Item = &'a T> + ExactSizeIterator> ExactSizeIterator
    for RefIter<'a, State, T, I>
{
}

#[derive(Debug)]
pub struct RefIterMut<'a, State: 'a + UnsafeClone, T: 'a, I: Iterator<Item = &'a mut T>> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: State,
    iter: I,
    _phantom: PhantomData<&'a State>,
}

impl<'a, State: 'a + UnsafeClone, T: 'a, I: Iterator<Item = &'a mut T>>
    RefIterMut<'a, State, T, I>
{
    #[inline(always)]
    pub fn new(borrow: State, iter: I) -> Self {
        Self {
            borrow,
            iter,
            _phantom: PhantomData,
        }
    }
}

impl<'a, State: 'a + UnsafeClone, T: 'a, I: Iterator<Item = &'a mut T>> Iterator
    for RefIterMut<'a, State, T, I>
{
    type Item = RefMut<'a, State, T>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.iter.next() {
            Some(RefMut::new(unsafe { self.borrow.clone() }, item))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) { self.iter.size_hint() }
}

impl<'a, State: 'a + UnsafeClone, T: 'a, I: Iterator<Item = &'a mut T> + ExactSizeIterator>
    ExactSizeIterator for RefIterMut<'a, State, T, I>
{
}
