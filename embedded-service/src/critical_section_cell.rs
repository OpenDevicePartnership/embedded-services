//! # CriticalSectionCell: a cell-like API for static interior mutability scenarios. Backed by a critical section, implying it's usage may delay or defer interrupts. Recommended to use sparingly.
use core::cell::Cell;

/// A critical section backed Cell for sync scenarios where you want Cell behaviors, but need it to be thread safe (such as used in statics). Backed by a critical section, use sparingly.
pub struct CriticalSectionCell<T: ?Sized> {
    inner: Cell<T>,
}

impl<T> CriticalSectionCell<T> {
    /// Constructs a CriticalSectionCell, initializing it with initial_value
    pub const fn new(initial_value: T) -> Self {
        Self {
            inner: Cell::new(initial_value),
        }
    }

    /// Sets the cell's content in a critical section. Note that this accounts
    /// for read/write conditions but does not automatically handle logical data
    /// race conditions. It is still possible for a user to read a value but have
    /// it change after they've performed the read. This just ensures data integrity:
    /// `CriticalSectionCell<T>` will always contain a valid `T`, even if it's been read "late"
    pub fn set(&self, value: T) {
        critical_section::with(|_cs| self.inner.set(value))
    }

    /// Swap contents between two CriticalSectionCell's
    /// # Panics
    ///
    /// This function will panic if `self` and `other` are different `Cell`s that partially overlap.
    /// (Using just standard library methods, it is impossible to create such partially overlapping `Cell`s.
    /// However, unsafe code is allowed to e.g. create two `&Cell<[i32; 2]>` that partially overlap.)
    pub fn swap(&self, other: &Self) {
        critical_section::with(|_cs| self.inner.swap(&other.inner));
    }

    /// consume the `CriticalSectionCell` and return the inner value `T`
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }
}

impl<T: Copy> CriticalSectionCell<T> {
    /// Reads the cell's content (in a critical section) and returns a copy
    pub fn get(&self) -> T {
        critical_section::with(|_cs| self.inner.get())
    }
}

impl<T: ?Sized> CriticalSectionCell<T> {
    /// Return an address to the backing type
    /// Unsafe: allows reads and writes without critical section guard, violating Sync guarantees.
    /// # Safety
    /// This may be used safely if and only if the pointer is held during a critical section, or
    /// all accessors to this Cell are blocked until the pointer is released.
    pub const fn as_ptr(&self) -> *mut T {
        self.inner.as_ptr()
    }
}

impl<T: Default> CriticalSectionCell<T> {
    /// consume the inner `T`, returning its value and replacing it with `Default::default()`
    pub fn take(&self) -> T {
        critical_section::with(|_cs| self.inner.take())
    }
}

// SAFETY: Sync is implemented here for `CriticalSectionCell` as `T` is only accessed via nestable critical sections
unsafe impl<T> Sync for CriticalSectionCell<T> {}

// SAFETY: Can implement send here due to critical section without T being explicitly Send
unsafe impl<T> Send for CriticalSectionCell<T> where T: Send {}

impl<T: Copy> Clone for CriticalSectionCell<T> {
    #[inline]
    fn clone(&self) -> CriticalSectionCell<T> {
        CriticalSectionCell::new(self.get())
    }
}

impl<T: Default> Default for CriticalSectionCell<T> {
    /// Creates a `Cell<T>`, with the `Default` value for `T`.
    #[inline]
    fn default() -> CriticalSectionCell<T> {
        CriticalSectionCell::new(Default::default())
    }
}

impl<T: PartialOrd + Copy> PartialOrd for CriticalSectionCell<T> {
    #[inline]
    fn partial_cmp(&self, other: &CriticalSectionCell<T>) -> Option<core::cmp::Ordering> {
        self.get().partial_cmp(&other.get())
    }

    #[inline]
    fn lt(&self, other: &CriticalSectionCell<T>) -> bool {
        self.get() < other.get()
    }

    #[inline]
    fn le(&self, other: &CriticalSectionCell<T>) -> bool {
        self.get() <= other.get()
    }

    #[inline]
    fn gt(&self, other: &CriticalSectionCell<T>) -> bool {
        self.get() > other.get()
    }

    #[inline]
    fn ge(&self, other: &CriticalSectionCell<T>) -> bool {
        self.get() >= other.get()
    }
}

impl<T: PartialEq + Copy> PartialEq for CriticalSectionCell<T> {
    #[inline]
    fn eq(&self, other: &CriticalSectionCell<T>) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq + Copy> Eq for CriticalSectionCell<T> {}

impl<T: Ord + Copy> Ord for CriticalSectionCell<T> {
    #[inline]
    fn cmp(&self, other: &CriticalSectionCell<T>) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl<T> From<T> for CriticalSectionCell<T> {
    /// Creates a new `CriticalSectionCell<T>` containing the given value.
    fn from(t: T) -> CriticalSectionCell<T> {
        CriticalSectionCell::new(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let sc = CriticalSectionCell::<()>::new(());

        // Ensure get() always returns the same type as the type the CriticalSectionCell was initialized with
        // This can be done statically at compile time
        let _: () = sc.get();
        sc.set(());
        let _: () = sc.get();
    }

    #[test]
    fn test_primitive() {
        let sc = CriticalSectionCell::new(0usize);

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

        let sc = CriticalSectionCell::new(Example { a: 0, b: 0 });

        assert_eq!(sc.get(), Example { a: 0, b: 0 });
        sc.set(Example { a: 1, b: 2 });
        assert_eq!(sc.get(), Example { a: 1, b: 2 });
    }

    #[tokio::test]
    async fn test_across_threads() {
        static SC: CriticalSectionCell<bool> = CriticalSectionCell::new(false);
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
            let _ = tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            scr.set(true);
        });

        let result = tokio::join!(poller, updater);
        assert!(result.0.is_ok());
        assert!(result.1.is_ok());

        assert!(SC.get());
    }
}
