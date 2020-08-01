//! A vector of slices.

use derivative::Derivative;
use std::iter::{FusedIterator, IntoIterator};

/// A vector of slices.
///
/// Each slice is stored inline so as to be efficiently iterated through linearly.
#[derive(Derivative, Debug)]
#[derivative(Default(bound = ""))]
pub struct SliceVec<T> {
    data: Vec<T>,
    counts: Vec<usize>,
    indices: Vec<usize>,
}

impl<T> SliceVec<T> {
    /// Pushes a new slice onto the end of the vector.
    pub fn push<I: IntoIterator<Item = T>>(&mut self, items: I) {
        self.indices.push(self.data.len());
        let mut count = 0;
        for item in items.into_iter() {
            self.data.push(item);
            count += 1;
        }
        self.counts.push(count);
    }

    /// Gets an iterator over slices starting from the given index.
    pub fn iter_from(&self, start: usize) -> SliceVecIter<T> {
        let index = *self.indices.get(start).unwrap_or(&self.data.len());
        SliceVecIter {
            data: &self.data[index..],
            counts: &self.counts[start..],
        }
    }
}

/// An iterator over slices in a `SliceVec`.
#[derive(Clone)]
pub struct SliceVecIter<'a, T> {
    pub(crate) data: &'a [T],
    pub(crate) counts: &'a [usize],
}

impl<'a, T> Iterator for SliceVecIter<'a, T> {
    type Item = &'a [T];

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some((count, remaining_counts)) = self.counts.split_first() {
            let (data, remaining_data) = self.data.split_at(*count);
            self.counts = remaining_counts;
            self.data = remaining_data;
            Some(data)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.counts.len(), Some(self.counts.len()))
    }

    #[inline]
    fn count(self) -> usize {
        self.len()
    }
}

impl<'a, T> ExactSizeIterator for SliceVecIter<'a, T> {}
impl<'a, T> FusedIterator for SliceVecIter<'a, T> {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create() {
        let _ = SliceVec::<usize>::default();
    }

    #[test]
    fn push() {
        let mut vec = SliceVec::default();
        let slices = [[1, 2, 3], [4, 5, 6]];

        for slice in &slices {
            vec.push(slice.iter().copied());
        }

        assert_eq!(vec.counts.len(), 2);
    }

    #[test]
    fn iter() {
        let mut vec = SliceVec::default();
        let slices = [[1, 2, 3], [4, 5, 6]];

        for slice in &slices {
            vec.push(slice.iter().copied());
        }

        assert_eq!(vec.counts.len(), 2);

        for (i, slice) in vec.iter_from(0).enumerate() {
            let expected = &slices[i];
            assert_eq!(slice.len(), expected.len());
            for (j, x) in slice.iter().enumerate() {
                assert_eq!(x, &expected[j]);
            }
        }
    }
}
