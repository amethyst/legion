use std::{iter::FusedIterator, marker::PhantomData};
#[derive(Clone)]
pub struct MapInto<I, T> {
    inner: I,
    _phantom: PhantomData<T>,
}

impl<I, T> MapInto<I, T> {
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }
}

impl<I, T> Iterator for MapInto<I, T>
where
    I: Iterator,
    T: From<I::Item>,
{
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(T::from)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }

    #[inline]
    fn count(self) -> usize {
        self.inner.count()
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.inner.nth(n).map(T::from)
    }

    #[inline]
    fn last(self) -> Option<Self::Item> {
        self.inner.last().map(T::from)
    }
}

impl<I, T> DoubleEndedIterator for MapInto<I, T>
where
    I: DoubleEndedIterator,
    T: From<I::Item>,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(T::from)
    }

    #[inline]
    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.inner.nth_back(n).map(T::from)
    }
}

impl<I, T> ExactSizeIterator for MapInto<I, T>
where
    I: ExactSizeIterator,
    T: From<I::Item>,
{
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<I, T> FusedIterator for MapInto<I, T>
where
    I: FusedIterator,
    T: From<I::Item>,
{
}
