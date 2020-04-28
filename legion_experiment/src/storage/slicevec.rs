use derivative::Derivative;
use std::iter::{FusedIterator, IntoIterator};

/// A vector of slices.
///
/// Each slice is stored inline so as to be efficiently iterated through linearly.
#[derive(Derivative)]
#[derivative(Default(bound = ""))]
pub struct SliceVec<T> {
    data: Vec<T>,
    counts: Vec<usize>,
}

impl<T> SliceVec<T> {
    /// Gets the length of the vector.
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// Determines if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len() < 1
    }

    /// Pushes a new slice onto the end of the vector.
    pub fn push<I: IntoIterator<Item = T>>(&mut self, items: I) {
        let mut count = 0;
        for item in items.into_iter() {
            self.data.push(item);
            count += 1;
        }
        self.counts.push(count);
    }

    /// Gets an iterator over all slices in the vector.
    pub fn iter(&self) -> SliceVecIter<T> {
        SliceVecIter {
            data: &self.data,
            counts: &self.counts,
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

        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn iter() {
        let mut vec = SliceVec::default();
        let slices = [[1, 2, 3], [4, 5, 6]];

        for slice in &slices {
            vec.push(slice.iter().copied());
        }

        assert_eq!(vec.len(), 2);

        for (i, slice) in vec.iter().enumerate() {
            let expected = &slices[i];
            assert_eq!(slice.len(), expected.len());
            for (j, x) in slice.iter().enumerate() {
                assert_eq!(x, &expected[j]);
            }
        }
    }
}
