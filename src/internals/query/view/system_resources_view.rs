#![doc(hidden)]

use super::ReadOnly;

use derivative::Derivative;
use std::marker::PhantomData;

/// TODO: better name?
#[derive(Derivative, Debug, Copy, Clone)]
#[derivative(Default(bound = ""))]
pub struct SystemResourcesView<T>(PhantomData<*const T>);

unsafe impl<T> Send for SystemResourcesView<T> {}
unsafe impl<T: Sync> Sync for SystemResourcesView<T> {}
unsafe impl<T> ReadOnly for SystemResourcesView<T> {}
