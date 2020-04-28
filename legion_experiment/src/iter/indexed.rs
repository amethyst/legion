use std::iter::FusedIterator;

pub unsafe trait TrustedRandomAccess: Sized {
    type Item;

    fn len(&self) -> usize;
    unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item;
    fn split_at(self, index: usize) -> (Self, Self);
}

unsafe impl<'a, T> TrustedRandomAccess for &'a [T] {
    type Item = &'a T;

    #[inline]
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    #[inline]
    unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item {
        &*self.as_ptr().add(i)
    }

    #[inline]
    fn split_at(self, index: usize) -> (Self, Self) {
        <[T]>::split_at(self, index)
    }
}

unsafe impl<'a, T> TrustedRandomAccess for &'a mut [T] {
    type Item = &'a mut T;

    #[inline]
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    #[inline]
    unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item {
        &mut *self.as_mut_ptr().add(i)
    }

    #[inline]
    fn split_at(self, index: usize) -> (Self, Self) {
        <[T]>::split_at_mut(self, index)
    }
}

#[derive(Clone, Debug)]
#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
pub struct IndexedIter<T> {
    inner: T,
    len: usize,
    index: usize,
}

impl<T: TrustedRandomAccess> IndexedIter<T> {
    pub fn new(inner: T) -> Self {
        let len = inner.len();
        Self {
            inner,
            len,
            index: 0,
        }
    }
}

impl<T: TrustedRandomAccess> Iterator for IndexedIter<T> {
    type Item = T::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.len {
            let i = self.index;
            self.index += 1;
            unsafe { Some(self.inner.get_unchecked(i)) }
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = <Self as ExactSizeIterator>::len(self);
        (len, Some(len))
    }

    #[inline]
    fn count(self) -> usize {
        <Self as ExactSizeIterator>::len(&self)
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let len = self.len();
        if n < len {
            self.index += n + 1;
            unsafe { Some(self.inner.get_unchecked(self.index - 1)) }
        } else {
            self.index = self.len;
            None
        }
    }

    #[inline]
    fn last(mut self) -> Option<Self::Item> {
        let len = <Self as ExactSizeIterator>::len(&self);
        if len > 0 {
            unsafe { Some(self.inner.get_unchecked(self.len - 1)) }
        } else {
            None
        }
    }
}

impl<T: TrustedRandomAccess> DoubleEndedIterator for IndexedIter<T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index < self.len {
            self.len -= 1;
            unsafe { Some(self.inner.get_unchecked(self.len)) }
        } else {
            None
        }
    }

    #[inline]
    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        let len = self.len();
        if n < len {
            self.len -= n + 1;
            unsafe { Some(self.inner.get_unchecked(self.len)) }
        } else {
            None
        }
    }
}

impl<T: TrustedRandomAccess> ExactSizeIterator for IndexedIter<T> {
    #[inline]
    fn len(&self) -> usize {
        self.len - self.index
    }
}

unsafe impl<T: TrustedRandomAccess> TrustedRandomAccess for IndexedIter<T> {
    type Item = <Self as Iterator>::Item;

    fn len(&self) -> usize {
        <Self as ExactSizeIterator>::len(self)
    }

    unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item {
        self.inner.get_unchecked(i)
    }

    fn split_at(self, index: usize) -> (Self, Self) {
        let (_, remaining) = self.inner.split_at(self.index);
        let (left, right) = remaining.split_at(index);
        (
            IndexedIter {
                index: 0,
                len: index,
                inner: left,
            },
            IndexedIter {
                index: 0,
                len: self.len - self.index - index,
                inner: right,
            },
        )
    }
}

impl<T: TrustedRandomAccess> FusedIterator for IndexedIter<T> {}

macro_rules! zip_slices {
    ($head_ty:ident) => {
        impl_zip_slices!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => (
        impl_zip_slices!($head_ty, $( $tail_ty ),*);
        zip_slices!($( $tail_ty ),*);
    );
}

macro_rules! impl_zip_slices {
    ( $( $ty: ident ),* ) => {
        paste::item! {
            #[allow(non_snake_case)]
            unsafe impl<$( $ty: TrustedRandomAccess ),*> TrustedRandomAccess for ($( $ty, )*) {
                type Item = ($( $ty::Item, )*);

                #[inline]
                fn len(&self) -> usize {
                    let len = std::usize::MAX;
                    let ($(ref $ty,)*) = self;
                    $( let len = std::cmp::min(len, $ty.len()); )*
                    len
                }

                #[inline]
                unsafe fn get_unchecked(&mut self, i: usize) -> Self::Item {
                    let ($(ref mut $ty,)*) = self;
                    ($( $ty.get_unchecked(i), )*)
                }

                fn split_at(self, index: usize) -> (Self, Self) {
                    let ($( $ty, )*) = self;
                    $( let ([<$ty _left>], [<$ty _right>]) = $ty.split_at(index); )*
                    (
                        ($( [<$ty _left>], )*),
                        ($( [<$ty _right>], )*),
                    )
                }
            }
        }
    };
}

zip_slices!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

// mod par_iter {
//     use super::{IndexedIter, TrustedRandomAccess};
//     use rayon::iter::plumbing::{
//         bridge_unindexed, Folder, Producer, UnindexedConsumer, UnindexedProducer,
//     };
//     use rayon::iter::{IndexedParallelIterator, ParallelIterator};

//     impl<T> Producer for IndexedIter<T>
//     where
//         T: TrustedRandomAccess + Send + Sync,
//     {
//         type Item = <Self as Iterator>::Item;
//         type IntoIter = Self;

//         fn into_iter(self) -> Self::IntoIter {
//             self
//         }

//         fn split_at(self, index: usize) -> (Self, Self) {
//             TrustedRandomAccess::split_at(self, index)
//         }
//     }

//     impl<T> UnindexedProducer for IndexedIter<T>
//     where
//         T: TrustedRandomAccess + Send + Sync,
//     {
//         type Item = <Self as Iterator>::Item;

//         fn split(self) -> (Self, Option<Self>) {
//             let len = ExactSizeIterator::len(&self);
//             let index = len / 2;
//             let (left, right) = TrustedRandomAccess::split_at(self, index);
//             (
//                 right,
//                 if ExactSizeIterator::len(&left) > 0 {
//                     Some(left)
//                 } else {
//                     None
//                 },
//             )
//         }

//         fn fold_with<F>(self, folder: F) -> F
//         where
//             F: Folder<Self::Item>,
//         {
//             folder.consume_iter(self)
//         }
//     }

//     impl<T> ParallelIterator for IndexedIter<T>
//     where
//         T: TrustedRandomAccess + Send + Sync,
//         <T as TrustedRandomAccess>::Item: Send,
//     {
//         type Item = T::Item;

//         fn drive_unindexed<C>(self, consumer: C) -> C::Result
//         where
//             C: UnindexedConsumer<Self::Item>,
//         {
//             bridge_unindexed(self, consumer)
//         }
//     }

//     impl<T> IndexedParallelIterator for IndexedIter<T>
//     where
//         T: TrustedRandomAccess + Send + Sync,
//         <T as TrustedRandomAccess>::Item: Send,
//     {
//         fn len(&self) -> usize {
//             ExactSizeIterator::len(self)
//         }
//     }
// }

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn iter_slice_single() {
        let values = vec![1, 2, 3, 4, 5];
        let slice = values.as_slice();
        let iter = IndexedIter::new((slice,));
        let collected: Vec<i32> = iter.map(|(x,)| *x).collect();

        assert_eq!(slice, collected.as_slice());
    }

    #[test]
    fn iter_slice_single_rev() {
        let values = vec![1, 2, 3, 4, 5];
        let slice = values.as_slice();
        let mut iter = IndexedIter::new((slice,));
        let mut collected = Vec::new();

        while let Some((x,)) = iter.next_back() {
            collected.insert(0, *x);
        }

        assert_eq!(slice, collected.as_slice());
    }

    #[test]
    fn iter_slice_single_empty() {
        let values: Vec<i32> = vec![];
        let slice = values.as_slice();
        let mut iter = IndexedIter::new((slice,));

        assert_eq!(iter.next(), None);
    }

    #[test]
    fn iter_slice_multiple() {
        let values_a: Vec<i32> = vec![1, 2, 3, 4, 5];
        let values_b: Vec<u32> = vec![1, 2, 3, 4, 5];
        let slice_a = values_a.as_slice();
        let slice_b = values_b.as_slice();
        let iter = IndexedIter::new((slice_a, slice_b));
        let collected: Vec<(&i32, &u32)> = iter.collect();

        assert_eq!(collected.len(), 5);
        for (i, (ref x, ref y)) in collected.iter().enumerate() {
            assert_eq!(&values_a[i], *x);
            assert_eq!(&values_b[i], *y);
        }
    }

    #[test]
    fn iter_slice_multiple_jagged() {
        let values_a: Vec<i32> = vec![1, 2, 3, 4, 5, 6, 7];
        let values_b: Vec<u32> = vec![1, 2, 3, 4, 5];
        let slice_a = values_a.as_slice();
        let slice_b = values_b.as_slice();
        let iter = IndexedIter::new((slice_a, slice_b));
        let collected: Vec<(&i32, &u32)> = iter.collect();

        assert_eq!(collected.len(), 5);
        for (i, (ref x, ref y)) in collected.iter().enumerate() {
            assert_eq!(&values_a[i], *x);
            assert_eq!(&values_b[i], *y);
        }
    }
}
