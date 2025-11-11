//! Synchronization utilities

use core::ops::DerefMut;

use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex};

/// General trait for types that allow locking to access an inner object
///
/// This trait allows code to be generic over multiple types that provide
/// interior mutability. This is primarily intended to allow using different
/// embassy mutex types and simplifies code by erasing the raw mutex type.
pub trait Lockable {
    /// Inner object type
    type Inner;

    /// Attempt to lock the inner object for mutable access
    fn try_lock(&self) -> Option<impl DerefMut<Target = Self::Inner>>;
    /// Lock the inner object for mutable access
    fn lock(&self) -> impl Future<Output = impl DerefMut<Target = Self::Inner>>;
}

impl<M: RawMutex, T> Lockable for Mutex<M, T> {
    type Inner = T;

    fn try_lock(&self) -> Option<impl DerefMut<Target = Self::Inner>> {
        self.try_lock().ok()
    }

    fn lock(&self) -> impl Future<Output = impl DerefMut<Target = Self::Inner>> {
        self.lock()
    }
}
