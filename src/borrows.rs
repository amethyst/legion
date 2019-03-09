use std::fmt::Debug;
use std::fmt::Display;
use std::ops::Deref;
use std::ops::DerefMut;
use std::slice::Iter;
use std::slice::IterMut;
use std::sync::atomic::{AtomicIsize, Ordering};

/// Tracks the borrowing of a piece of memory.
pub enum Borrow<'a> {
    Read { state: &'a AtomicIsize },
    Write { state: &'a AtomicIsize },
}

impl<'a> Borrow<'a> {
    /// Attempts to aquire a readonly borrow.
    pub fn aquire_read(state: &'a AtomicIsize) -> Result<Borrow<'a>, &'static str> {
        loop {
            let read = state.load(Ordering::SeqCst);
            if read < 0 {
                return Err("resource already borrowed as mutable");
            }

            if state.compare_and_swap(read, read + 1, Ordering::SeqCst) == read {
                break;
            }
        }

        Ok(Borrow::Read { state })
    }

    /// Attempts to aquire a mutable borrow.
    pub fn aquire_write(state: &'a AtomicIsize) -> Result<Borrow<'a>, &'static str> {
        let borrowed = state.compare_and_swap(0, -1, Ordering::SeqCst);
        match borrowed {
            0 => Ok(Borrow::Write { state }),
            x if x < 0 => Err("resource already borrowed as mutable"),
            _ => Err("resource already borrowed as immutable"),
        }
    }
}

impl<'a> Drop for Borrow<'a> {
    fn drop(&mut self) {
        match *self {
            Borrow::Read { state } => {
                state.fetch_sub(1, Ordering::SeqCst);
            }
            Borrow::Write { state } => {
                state.store(0, Ordering::SeqCst);
            }
        };
    }
}

/// Represents a piece of runtime borrow checked data.
pub struct Borrowed<'a, T: 'a> {
    value: &'a T,
    #[allow(dead_code)]
    // held for drop impl
    state: Borrow<'a>,
}

impl<'a, T: 'a> Borrowed<'a, T> {
    /// Constructs a new `Borrowed<'a, T>`.
    pub fn new(value: &'a T, borrow: Borrow<'a>) -> Borrowed<'a, T> {
        Borrowed {
            value,
            state: borrow,
        }
    }
}

impl<'a, 'b, T: 'a + Debug> Debug for Borrowed<'a, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.value.fmt(formatter)
    }
}

impl<'a, 'b, T: 'a + Display> Display for Borrowed<'a, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.value.fmt(formatter)
    }
}

impl<'a, 'b, T: 'a + PartialEq<T>> PartialEq<Borrowed<'b, T>> for Borrowed<'a, T> {
    fn eq(&self, other: &Borrowed<'b, T>) -> bool {
        self.value.eq(other.value)
    }
}

impl<'a, 'b, T: 'a + PartialEq<T>> PartialEq<T> for Borrowed<'a, T> {
    fn eq(&self, other: &T) -> bool {
        self.value.eq(other)
    }
}

impl<'a, 'b, T: 'a + Eq> Eq for Borrowed<'a, T> {}

impl<'a, T: 'a> Deref for Borrowed<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T: 'a> AsRef<T> for Borrowed<'a, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for Borrowed<'a, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}

/// Represents a piece of mutable runtime borrow checked data.
pub struct BorrowedMut<'a, T: 'a> {
    value: &'a mut T,
    #[allow(dead_code)]
    // held for drop impl
    state: Borrow<'a>,
}

impl<'a, T: 'a> BorrowedMut<'a, T> {
    /// Constructs a new `Borrowedmut<'a, T>`.
    pub fn new(value: &'a mut T, borrow: Borrow<'a>) -> BorrowedMut<'a, T> {
        BorrowedMut {
            value,
            state: borrow,
        }
    }
}

impl<'a, T: 'a> Deref for BorrowedMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T: 'a> DerefMut for BorrowedMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, T: 'a> AsRef<T> for BorrowedMut<'a, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

impl<'a, T: 'a> std::borrow::Borrow<T> for BorrowedMut<'a, T> {
    fn borrow(&self) -> &T {
        self.value
    }
}

impl<'a, 'b, T: 'a + Debug> Debug for BorrowedMut<'a, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.value.fmt(formatter)
    }
}

impl<'a, 'b, T: 'a + Display> Display for BorrowedMut<'a, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.value.fmt(formatter)
    }
}

/// Represents a runtime borrow checked slice.
pub struct BorrowedSlice<'a, T: 'a> {
    slice: &'a [T],
    state: Borrow<'a>,
}

impl<'a, T: 'a> BorrowedSlice<'a, T> {
    /// Constructs a new `BorrowedSlice<'a, T>`.
    pub fn new(slice: &'a [T], borrow: Borrow<'a>) -> BorrowedSlice<'a, T> {
        BorrowedSlice {
            slice,
            state: borrow,
        }
    }

    /// Borrows a single element from the slice.
    pub fn single(self, i: usize) -> Option<Borrowed<'a, T>> {
        let slice = self.slice;
        let state = self.state;
        slice.get(i).map(|x| Borrowed::new(x, state))
    }
}

impl<'a, T: 'a> Deref for BorrowedSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.slice
    }
}

impl<'a, T: 'a> IntoIterator for BorrowedSlice<'a, T> {
    type Item = &'a T;
    type IntoIter = BorrowedIter<'a, Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        BorrowedIter {
            inner: self.slice.into_iter(),
            state: self.state,
        }
    }
}

/// Represents a runtime borrow checked mut slice.
pub struct BorrowedMutSlice<'a, T: 'a> {
    slice: &'a mut [T],
    state: Borrow<'a>,
}

impl<'a, T: 'a> BorrowedMutSlice<'a, T> {
    /// Constructs a new `BorrowedMutSlice<'a, T>`.
    pub fn new(slice: &'a mut [T], borrow: Borrow<'a>) -> BorrowedMutSlice<'a, T> {
        BorrowedMutSlice {
            slice,
            state: borrow,
        }
    }

    /// Borrows a single element from the slice.
    pub fn single(self, i: usize) -> Option<BorrowedMut<'a, T>> {
        let slice = self.slice;
        let state = self.state;
        slice.get_mut(i).map(|x| BorrowedMut::new(x, state))
    }
}

impl<'a, T: 'a> Deref for BorrowedMutSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.slice
    }
}

impl<'a, T: 'a> DerefMut for BorrowedMutSlice<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.slice
    }
}

impl<'a, T: 'a> IntoIterator for BorrowedMutSlice<'a, T> {
    type Item = &'a mut T;
    type IntoIter = BorrowedIter<'a, IterMut<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        BorrowedIter {
            inner: self.slice.into_iter(),
            state: self.state,
        }
    }
}

/// Represents a runtime borrow checked iterator.
pub struct BorrowedIter<'a, I: 'a + Iterator> {
    inner: I,
    #[allow(dead_code)]
    // held for drop impl
    state: Borrow<'a>,
}

impl<'a, I: 'a + Iterator> Iterator for BorrowedIter<'a, I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, I: 'a + ExactSizeIterator> ExactSizeIterator for BorrowedIter<'a, I> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicIsize, Ordering};

    #[test]
    fn borrow_read() {
        let state = AtomicIsize::new(0);
        let x = 5u8;

        let _borrow = Borrowed::new(&x, Borrow::aquire_read(&state).unwrap());
        assert_eq!(1, state.load(Ordering::SeqCst));
    }

    #[test]
    fn drop_read() {
        let state = AtomicIsize::new(0);
        let x = 5u8;

        {
            let _borrow = Borrowed::new(&x, Borrow::aquire_read(&state).unwrap());
            assert_eq!(1, state.load(Ordering::SeqCst));
        }

        assert_eq!(0, state.load(Ordering::SeqCst));
    }

    #[test]
    fn borrow_write() {
        let state = AtomicIsize::new(0);
        let x = 5u8;

        let _borrow = Borrowed::new(&x, Borrow::aquire_write(&state).unwrap());
        assert_eq!(-1, state.load(Ordering::SeqCst));
    }

    #[test]
    fn drop_write() {
        let state = AtomicIsize::new(0);
        let x = 5u8;

        {
            let _borrow = Borrowed::new(&x, Borrow::aquire_write(&state).unwrap());
            assert_eq!(-1, state.load(Ordering::SeqCst));
        }

        assert_eq!(0, state.load(Ordering::SeqCst));
    }

    #[test]
    fn read_while_reading() {
        let state = AtomicIsize::new(0);

        let _read = Borrow::aquire_read(&state).unwrap();
        let _read2 = Borrow::aquire_read(&state).unwrap();
    }

    #[test]
    #[should_panic(expected = "resource already borrowed as immutable")]
    fn write_while_reading() {
        let state = AtomicIsize::new(0);

        let _read = Borrow::aquire_read(&state).unwrap();
        let _write = Borrow::aquire_write(&state).unwrap();
    }

    #[test]
    #[should_panic(expected = "resource already borrowed as mutable")]
    fn read_while_writing() {
        let state = AtomicIsize::new(0);

        let _write = Borrow::aquire_write(&state).unwrap();
        let _read = Borrow::aquire_read(&state).unwrap();
    }

    #[test]
    #[should_panic(expected = "resource already borrowed as mutable")]
    fn write_while_writing() {
        let state = AtomicIsize::new(0);

        let _write = Borrow::aquire_write(&state).unwrap();
        let _write2 = Borrow::aquire_write(&state).unwrap();
    }
}
