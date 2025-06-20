//! # SyncCell: a cell-like API for static interior mutability scenarios
use core::cell::UnsafeCell;

/// A critical section backed Cell for sync scenarios where you want Cell behaviors, but need it to be thread safe (such as used in statics)
pub struct SyncCell<T: Copy> {
    inner: UnsafeCell<T>,
}

impl<T: Copy> SyncCell<T> {
    /// Constructs a SyncCell, initializing it with initial_value
    pub const fn new(initial_value: T) -> Self {
        Self {
            inner: UnsafeCell::new(initial_value),
        }
    }

    /// Reads the cell's content (in a critical section) and returns a copy
    pub fn get(&self) -> T {
        critical_section::with(|_cs|
            // SAFETY: safe as accessors (get/set) are always completed in a critical section
            unsafe {
            *self.inner.get()
        })
    }

    /// Sets the cell's content in a critical section. Note that this accounts
    /// for read/write conditions but does not automatically handle logical data
    /// race conditions. It is still possible for a user to read a value but have
    /// it change after they've performed the read. This just ensures data integrity:
    /// SyncCell<T> will always contain a valid T, even if it's been read "late"
    pub fn set(&self, value: T) {
        critical_section::with(|_cs|
            // SAFETY: safe as accessors (get/set) are always completed in a critical section
            unsafe {
                *self.inner.get() = value;
        })
    }

    /// Unsafe: allows reads and writes without critical section guard, violating Sync guarantees.
    /// # Safety
    /// This may be used safely if and only if the pointer is held during a critical section, or
    /// all accessors to this Cell are blocked until the pointer is released.
    pub unsafe fn as_ptr(&self) -> *mut T {
        self.inner.get()
    }
}

// SAFETY: Sync is implemented here for SyncCell as T is only accessed via nestable critical sections
unsafe impl<T: Copy> Sync for SyncCell<T> {}

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
