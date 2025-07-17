//! Object trait and related implementations

use core::ops::{Deref, DerefMut};

use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    mutex::{Mutex, MutexGuard},
};

/// Trait to allow for borrowing a reference to the inner type
pub trait RefGuard<Inner>: Deref<Target = Inner> {}

/// Trait to allow for borrowing a mutable reference to the inner type
pub trait RefMutGuard<Inner>: DerefMut<Target = Inner> {}

/// Object trait
pub trait Object<Inner> {
    /// Get a reference to the inner object
    fn get_inner(&self) -> impl Future<Output = impl RefGuard<Inner>>;
    /// Get a mutable reference to the inner object
    fn get_inner_mut(&self) -> impl Future<Output = impl RefMutGuard<Inner>>;
}

/// A mutex wrapped object
pub struct ObjectMutex<Inner, M: RawMutex> {
    inner: Mutex<M, Inner>,
}

impl<Inner, M: RawMutex> ObjectMutex<Inner, M> {
    /// Create a new ObjectMutex
    pub fn new(inner: Inner) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl<Inner, M: RawMutex> Object<Inner> for ObjectMutex<Inner, M> {
    /// Get a reference to the inner object
    async fn get_inner(&self) -> impl RefGuard<Inner> {
        self.inner.lock().await
    }

    /// Get a mutable reference to the inner object
    async fn get_inner_mut(&self) -> impl RefMutGuard<Inner> {
        self.inner.lock().await
    }
}

impl<Inner, M: RawMutex> RefGuard<Inner> for MutexGuard<'_, M, Inner> {}
impl<Inner, M: RawMutex> RefMutGuard<Inner> for MutexGuard<'_, M, Inner> {}
