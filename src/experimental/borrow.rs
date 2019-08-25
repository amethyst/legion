use std::cell::UnsafeCell;
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

    pub fn get<'a>(&'a self) -> Ref<'a, T> {
        self.try_get().unwrap()
    }

    pub fn try_get<'a>(&'a self) -> Result<Ref<'a, T>, &'static str> {
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

        Ok(Ref {
            borrow: Borrow {
                state: &self.borrow_state,
            },
            value: unsafe { &*self.value.get() },
        })
    }

    pub fn get_mut<'a>(&'a self) -> RefMut<'a, T> {
        self.try_get_mut().unwrap()
    }

    pub fn try_get_mut<'a>(&'a self) -> Result<RefMut<'a, T>, &'static str> {
        let borrowed = self.borrow_state.compare_and_swap(0, -1, Ordering::SeqCst);
        match borrowed {
            0 => Ok(RefMut {
                borrow: BorrowMut {
                    state: &self.borrow_state,
                },
                value: unsafe { &mut *self.value.get() },
            }),
            x if x < 0 => Err("resource already borrowed as mutable"),
            _ => Err("resource already borrowed as immutable"),
        }
    }
}

#[derive(Debug)]
pub struct Borrow<'a> {
    state: &'a AtomicIsize,
}

impl<'a> Drop for Borrow<'a> {
    fn drop(&mut self) {
        self.state.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<'a> Clone for Borrow<'a> {
    fn clone(&self) -> Self {
        self.state.fetch_add(1, Ordering::SeqCst);
        Borrow { state: self.state }
    }
}

#[derive(Debug)]
pub struct BorrowMut<'a> {
    state: &'a AtomicIsize,
}

impl<'a> Drop for BorrowMut<'a> {
    fn drop(&mut self) {
        self.state.store(0, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct Ref<'a, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: Borrow<'a>,
    value: &'a T,
}

impl<'a, T: 'a> Clone for Ref<'a, T> {
    fn clone(&self) -> Self {
        Ref {
            borrow: self.borrow.clone(),
            value: self.value,
        }
    }
}

impl<'a, T: 'a> Ref<'a, T> {
    pub fn new(borrow: Borrow<'a>, value: &'a T) -> Self {
        Self { borrow, value }
    }

    pub fn map_into<K: 'a, F: FnMut(&T) -> K>(self, mut f: F) -> RefMap<'a, K> {
        RefMap {
            value: f(&self.value),
            borrow: self.borrow,
        }
    }

    pub unsafe fn deconstruct(self) -> (Borrow<'a>, &'a T) {
        (self.borrow, self.value)
    }
}

impl<'a, T: 'a> Deref for Ref<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T: 'a> AsRef<T> for Ref<'a, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for Ref<'a, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}

#[derive(Debug)]
pub struct RefMut<'a, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: BorrowMut<'a>,
    value: &'a mut T,
}

impl<'a, T: 'a> RefMut<'a, T> {
    pub fn new(borrow: BorrowMut<'a>, value: &'a mut T) -> Self {
        Self { borrow, value }
    }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, K> {
        RefMapMut {
            value: f(&mut self.value),
            borrow: self.borrow,
        }
    }

    pub unsafe fn deconstruct(self) -> (BorrowMut<'a>, &'a mut T) {
        (self.borrow, self.value)
    }
}

impl<'a, T: 'a> Deref for RefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T: 'a> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, T: 'a> AsRef<T> for RefMut<'a, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for RefMut<'a, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}

#[derive(Debug)]
pub struct RefMap<'a, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: Borrow<'a>,
    value: T,
}

impl<'a, T: 'a> RefMap<'a, T> {
    pub fn new(borrow: Borrow<'a>, value: T) -> Self {
        Self { borrow, value }
    }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMap<'a, K> {
        RefMap {
            value: f(&mut self.value),
            borrow: self.borrow,
        }
    }

    pub unsafe fn deconstruct(self) -> (Borrow<'a>, T) {
        (self.borrow, self.value)
    }
}

impl<'a, T: 'a> Deref for RefMap<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T: 'a> AsRef<T> for RefMap<'a, T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for RefMap<'a, T> {
    fn borrow(&self) -> &T {
        &self.value
    }
}

impl<'a, I: 'a + Iterator> Iterator for RefMap<'a, I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.value.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.value.size_hint()
    }
}

impl<'a, I: 'a + ExactSizeIterator> ExactSizeIterator for RefMap<'a, I> {}

#[derive(Debug)]
pub struct RefMapMut<'a, T: 'a> {
    #[allow(dead_code)]
    // held for drop impl
    borrow: BorrowMut<'a>,
    value: T,
}

impl<'a, T: 'a> RefMapMut<'a, T> {
    pub fn new(borrow: BorrowMut<'a>, value: T) -> Self {
        Self { borrow, value }
    }

    pub fn map_into<K: 'a, F: FnMut(&mut T) -> K>(mut self, mut f: F) -> RefMapMut<'a, K> {
        RefMapMut {
            value: f(&mut self.value),
            borrow: self.borrow,
        }
    }

    pub unsafe fn deconstruct(self) -> (BorrowMut<'a>, T) {
        (self.borrow, self.value)
    }
}

impl<'a, T: 'a> Deref for RefMapMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T: 'a> DerefMut for RefMapMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<'a, T: 'a> AsRef<T> for RefMapMut<'a, T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for RefMapMut<'a, T> {
    fn borrow(&self) -> &T {
        &self.value
    }
}

impl<'a, I: 'a + Iterator> Iterator for RefMapMut<'a, I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.value.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.value.size_hint()
    }
}

impl<'a, I: 'a + ExactSizeIterator> ExactSizeIterator for RefMapMut<'a, I> {}

#[derive(Debug)]
pub struct ComponentRef<'a, 'b: 'a, T: 'b> {
    #[allow(dead_code)]
    // held for drop impl
    arch_borrow: Borrow<'a>,
    #[allow(dead_code)]
    // held for drop impl
    chunk_borrow: Borrow<'b>,
    value: &'b T,
}

impl<'a, 'b: 'a, T: 'b> ComponentRef<'a, 'b, T> {
    pub fn new(arch_borrow: Borrow<'a>, chunk_borrow: Borrow<'b>, value: &'b T) -> Self {
        Self {
            arch_borrow,
            chunk_borrow,
            value,
        }
    }
}

impl<'a, 'b: 'a, T: 'b> Deref for ComponentRef<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, 'b: 'a, T: 'b> AsRef<T> for ComponentRef<'a, 'b, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, 'b: 'a, T: 'b> std::borrow::Borrow<T> for ComponentRef<'a, 'b, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}

#[derive(Debug)]
pub struct ComponentRefMut<'a, 'b: 'a, T: 'b> {
    #[allow(dead_code)]
    // held for drop impl
    arch_borrow: Borrow<'a>,
    #[allow(dead_code)]
    // held for drop impl
    chunk_borrow: BorrowMut<'b>,
    value: &'b mut T,
}

impl<'a, 'b: 'a, T: 'b> ComponentRefMut<'a, 'b, T> {
    pub fn new(arch_borrow: Borrow<'a>, chunk_borrow: BorrowMut<'b>, value: &'b mut T) -> Self {
        Self {
            arch_borrow,
            chunk_borrow,
            value,
        }
    }
}

impl<'a, 'b: 'a, T: 'b> Deref for ComponentRefMut<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, 'b: 'a, T: 'b> DerefMut for ComponentRefMut<'a, 'b, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, 'b: 'a, T: 'b> AsRef<T> for ComponentRefMut<'a, 'b, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, 'b: 'a, T: 'b> std::borrow::Borrow<T> for ComponentRefMut<'a, 'b, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}
