//! # SyncCell: a cell-like API for static interior mutability scenarios
use core::cell::Cell;

/// A critical section backed Cell for sync scenarios where you want Cell behaviors, but need it to be thread safe (such as used in statics)
pub struct SyncCell<T: ?Sized> {
    inner: Cell<T>,
}

impl<T> SyncCell<T> {
    /// Constructs a SyncCell, initializing it with initial_value
    pub const fn new(initial_value: T) -> Self {
        Self {
            inner: Cell::new(initial_value),
        }
    }

    /// Sets the cell's content in a critical section. Note that this accounts
    /// for read/write conditions but does not automatically handle logical data
    /// race conditions. It is still possible for a user to read a value but have
    /// it change after they've performed the read. This just ensures data integrity:
    /// SyncCell<T> will always contain a valid T, even if it's been read "late"
    pub fn set(&self, value: T) {
        critical_section::with(|_cs| self.inner.set(value))
    }

    /// Swap contents between two SyncCell's
    pub fn swap(&self, other: &Self) {
        critical_section::with(|_cs| self.inner.swap(&other.inner));
    }

    // we do not implement replace() as we cannot guarantee that it will not be called outside of compile time

    /// consume the Synccell and return the inner value T
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }
}

impl<T: Copy> SyncCell<T> {
    /// Reads the cell's content (in a critical section) and returns a copy
    pub fn get(&self) -> T {
        critical_section::with(|_cs| self.inner.get())
    }
}

impl<T: ?Sized> SyncCell<T> {
    /// Unsafe: allows reads and writes without critical section guard, violating Sync guarantees.
    /// Note that this is marked unsafe, whereas as_ptr() over a standard Cell<> is safe. This is because
    /// Cell<> implements !Sync, whereas we are implementing Sync.
    /// # Safety
    /// This may be used safely if and only if the pointer is held during a critical section, or
    /// all accessors to this Cell are blocked until the pointer is released.
    pub const unsafe fn as_ptr(&self) -> *mut T {
        self.inner.as_ptr()
    }
}

impl<T: Default> SyncCell<T> {
    /// consume the inner T, returning its value and replacing it with default()
    pub fn take(&self) -> T {
        critical_section::with(|_cs| self.inner.take())
    }
}

// SAFETY: Sync is implemented here for SyncCell as T is only accessed via nestable critical sections
unsafe impl<T> Sync for SyncCell<T> {}

// SAFETY: Can implement send here due to critical section without T being explicitly Send
unsafe impl<T> Send for SyncCell<T> where T:Send {}

impl<T: Copy> Clone for SyncCell<T> {
    #[inline]
    fn clone(&self) -> SyncCell<T> {
        SyncCell::new(self.get())
    }
}

impl<T: Default> Default for SyncCell<T> {
    /// Creates a `Cell<T>`, with the `Default` value for T.
    #[inline]
    fn default() -> SyncCell<T> {
        SyncCell::new(Default::default())
    }
}

impl<T: PartialOrd + Copy> PartialOrd for SyncCell<T> {
    #[inline]
    fn partial_cmp(&self, other: &SyncCell<T>) -> Option<core::cmp::Ordering> {
        self.get().partial_cmp(&other.get())
    }

    #[inline]
    fn lt(&self, other: &SyncCell<T>) -> bool {
        self.get() < other.get()
    }

    #[inline]
    fn le(&self, other: &SyncCell<T>) -> bool {
        self.get() <= other.get()
    }

    #[inline]
    fn gt(&self, other: &SyncCell<T>) -> bool {
        self.get() > other.get()
    }

    #[inline]
    fn ge(&self, other: &SyncCell<T>) -> bool {
        self.get() >= other.get()
    }
}

impl<T: PartialEq + Copy> PartialEq for SyncCell<T> {
    #[inline]
    fn eq(&self, other: &SyncCell<T>) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq + Copy> Eq for SyncCell<T> {}

impl<T: Ord + Copy> Ord for SyncCell<T> {
    #[inline]
    fn cmp(&self, other: &SyncCell<T>) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl<T> From<T> for SyncCell<T> {
    /// Creates a new `SyncCell<T>` containing the given value.
    fn from(t: T) -> SyncCell<T> {
        SyncCell::new(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let sc = SyncCell::<()>::new(());

        assert_eq!(sc.get(), ());
        sc.set(());
        assert_eq!(sc.get(), ());
    }

    #[test]
    fn test_primitive() {
        let sc = SyncCell::new(0usize);

        assert_eq!(sc.get(), 0);
        sc.set(1);
        assert_eq!(sc.get(), 1);
    }

    #[test]
    fn test_struct() {
        #[derive(Copy, Clone, PartialEq, Debug)]
        struct Example {
            a: u32,
            b: u32,
        }

        let sc = SyncCell::new(Example { a: 0, b: 0 });

        assert_eq!(sc.get(), Example { a: 0, b: 0 });
        sc.set(Example { a: 1, b: 2 });
        assert_eq!(sc.get(), Example { a: 1, b: 2 });
    }

    #[tokio::test]
    async fn test_across_threads() {
        static SC: SyncCell<bool> = SyncCell::new(false);
        let scr = &SC;

        let poller = tokio::spawn(async {
            loop {
                if scr.get() {
                    break;
                } else {
                    let _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        });

        let updater = tokio::spawn(async {
            let _ = tokio::time::sleep(tokio::time::Duration::from_millis(300));
            scr.set(true);
        });

        let result = tokio::join!(poller, updater);
        assert!(result.0.is_ok());
        assert!(result.1.is_ok());

        assert!(SC.get());
    }
}
