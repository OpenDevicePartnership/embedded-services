//! # ThreadModeCell: A cell-like API for static interior mutability scenarios. Backed by thread mode assertion, implying this is only safe on single-core systems.
use core::cell::Cell;
use cortex_m::peripheral::{scb::VectActive, SCB};

#[inline(always)]
fn in_thread_mode() -> bool {
    SCB::vect_active() == VectActive::ThreadMode
}

/// A Sync-safe Cell backed by a lightweight assertion that it is only being accessed in thread mode.
///
/// # Safety
///
/// Attempting to access `ThreadModeCell` within an interrupt context will cause a panic.
///
/// **`ThreadModeCell` is only safe on single-core systems.**
/// On multi-core systems, a `ThreadModeCell` **is not sufficient** to ensure exclusive access.
pub struct ThreadModeCell<T: ?Sized> {
    inner: Cell<T>,
}

impl<T> ThreadModeCell<T> {
    /// Constructs a `ThreadModeCell`, initializing it with initial_value.
    pub const fn new(initial_value: T) -> Self {
        Self {
            inner: Cell::new(initial_value),
        }
    }

    /// Sets the `ThreadModeCell`'s content.
    /// # Note
    /// This does **NOT** protect against general race conditions, only data races.
    ///
    /// For example, even in a single execution environment, two cooperating tasks might still
    /// overwrite the cell with outdated data.
    ///
    /// Use `Self::update` to guarantee protection against race conditions.
    /// # Panics
    ///
    /// This function will panic if called from within an interrupt context.
    pub fn set(&self, value: T) {
        assert!(in_thread_mode(), "ThreadModeCell can only be accessed in thread mode.");
        self.inner.set(value)
    }

    /// Swap contents between two `ThreadModeCell`'s.
    /// # Panics
    ///
    /// This function will panic if `self` and `other` are different `Cell`s that partially overlap.
    /// (Using just standard library methods, it is impossible to create such partially overlapping `Cell`s.
    /// However, unsafe code is allowed to e.g. create two `&Cell<[i32; 2]>` that partially overlap.)
    ///
    /// This function will panic if called from within an interrupt context.
    pub fn swap(&self, other: &Self) {
        assert!(in_thread_mode(), "ThreadModeCell can only be accessed in thread mode.");
        self.inner.swap(&other.inner);
    }

    /// Consume the `ThreadModeCell` and return the inner value T.
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }
}

impl<T: Copy> ThreadModeCell<T> {
    /// Reads the cell's content and returns a copy.
    /// # Panics
    ///
    /// This function will panic if called from within an interrupt context.
    pub fn get(&self) -> T {
        assert!(in_thread_mode(), "ThreadModeCell can only be accessed in thread mode.");
        self.inner.get()
    }

    /// Updates the `ThreadModeCell`'s content using a function.
    ///
    /// This guarantees race conditions will not occur as this can only be called in a single
    /// execution environment (thread mode) with cooperative scheduling.
    /// # Panics
    ///
    /// This function will panic if called from within an interrupt context.
    pub fn update(&self, f: impl FnOnce(T) -> T) {
        assert!(in_thread_mode(), "ThreadModeCell can only be accessed in thread mode.");
        self.inner.update(f)
    }
}

impl<T: ?Sized> ThreadModeCell<T> {
    /// Return an address to the backing type.
    /// Unsafe: allows reads and writes without thread mode assertion, violating Sync guarantees.
    ///
    /// # Safety
    ///
    /// This may be used safely if and only if the pointer is used in thread mode.
    pub const fn as_ptr(&self) -> *mut T {
        self.inner.as_ptr()
    }
}

impl<T: Default> ThreadModeCell<T> {
    /// Consume the inner T, returning its value and replacing it with default().
    /// # Panics
    ///
    /// This function will panic if called from within an interrupt context.
    pub fn take(&self) -> T {
        assert!(in_thread_mode(), "ThreadModeCell can only be accessed in thread mode.");
        self.inner.take()
    }
}

// SAFETY: Sync is implemented here for ThreadModeCell as T is only accessed in a single-core, thread mode context.
unsafe impl<T> Sync for ThreadModeCell<T> {}

// Although ideally T shouldn't need to be Send since we are only operating in a single context,
// the possibility exists that T could sent to interrupt context then dropped.
// Implementing Drop for this type which does the thread-mode check is difficult,
// so restrict T to Send to be on the safe-side.
// SAFETY: `ThreadModeCell` is only accessed in a single execution context.
unsafe impl<T> Send for ThreadModeCell<T> where T: Send {}

impl<T: Copy> Clone for ThreadModeCell<T> {
    #[inline]
    fn clone(&self) -> ThreadModeCell<T> {
        ThreadModeCell::new(self.get())
    }
}

impl<T: Default> Default for ThreadModeCell<T> {
    /// Creates a `ThreadModeCell<T>`, with the `Default` value for T.
    #[inline]
    fn default() -> ThreadModeCell<T> {
        ThreadModeCell::new(Default::default())
    }
}

impl<T: PartialOrd + Copy> PartialOrd for ThreadModeCell<T> {
    #[inline]
    fn partial_cmp(&self, other: &ThreadModeCell<T>) -> Option<core::cmp::Ordering> {
        self.get().partial_cmp(&other.get())
    }
}

impl<T: PartialEq + Copy> PartialEq for ThreadModeCell<T> {
    #[inline]
    fn eq(&self, other: &ThreadModeCell<T>) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq + Copy> Eq for ThreadModeCell<T> {}

impl<T: Ord + Copy> Ord for ThreadModeCell<T> {
    #[inline]
    fn cmp(&self, other: &ThreadModeCell<T>) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl<T> From<T> for ThreadModeCell<T> {
    /// Creates a new `ThreadModeCell<T>` containing the given value.
    fn from(t: T) -> ThreadModeCell<T> {
        ThreadModeCell::new(t)
    }
}
