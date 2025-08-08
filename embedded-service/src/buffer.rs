//! Efficiently passing large amounts of data between components is best done by passing references to a buffer.
//! However, async code generally requires 'static lifetimes on references. Buffers generally also need
//! to be mutable. This module provides a way to manage ownership and access to buffers, particulary those with 'static lifetimes.
//!
//! This modules provides `OwnedRef` and `SharedSlice` types. `OwnedRef` represents ownership of the underlying buffer
//! and allows mutable access to the buffer. This type does not implement `Copy` or `Clone` so as to provide compile-time
//! ownership guarantees. `SharedRef` represents an immutable reference into the buffer. This type can be cloned
//! and can be created from an `OwnedRef`. `Access` and `AccessMut` are guard types that provide access to the buffer through
//! references tied to the lifetime of the guard struct. These types enforce Rust's aliasing and mutability rules dynamically,
//! similar to RefCell.
//!
//! This allows for producer code to own the buffer through a `OwnedRef`, and then allow access to consumers
//! through any number of `SharedRef`.
use core::borrow::{Borrow, BorrowMut};
use core::marker::PhantomData;
use core::ops::Range;

use crate::SyncCell;
use core::sync::atomic::AtomicPtr;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Status {
    None,
    Mutable,
    Immutable(u32),
}

/// Underlying buffer storage struct
pub struct Buffer<'a, T> {
    buffer: AtomicPtr<T>,
    len: usize,
    status: SyncCell<Status>,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, T> Buffer<'a, T> {
    /// Create a new buffer from a reference
    /// # Safety
    /// No other code should have access to the buffer
    pub unsafe fn new(raw_buffer: &'a mut [T]) -> Self {
        Buffer {
            buffer: AtomicPtr::new(raw_buffer.as_mut_ptr()),
            len: raw_buffer.len(),
            status: SyncCell::new(Status::None),
            _lifetime: PhantomData,
        }
    }

    /// Create an owned reference to the buffer
    /// # Safety
    /// Can be used to create mulitple mut references to the buffer
    pub unsafe fn as_owned(&'a self) -> OwnedRef<'a, T> {
        OwnedRef(self)
    }

    /// Returns the length of the buffer
    // SAFETY: The buffer is always valid
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true`` if the buffer has a length of 0
    // SAFETY: The buffer is always valid
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn borrow(&self, mutable: bool) {
        let status = match (self.status.get(), mutable) {
            (Status::None, false) => Status::Immutable(1),
            (Status::None, true) => Status::Mutable,
            (Status::Mutable, _) => panic!("Buffer already borrowed mutably"),
            (Status::Immutable(count), false) => Status::Immutable(count + 1),
            (Status::Immutable(_), true) => panic!("Buffer already borrowed immutably"),
        };
        self.status.set(status);
    }

    fn drop_borrow(&self) {
        let status = match self.status.get() {
            Status::None => panic!("Unborrowed buffer dropped"),
            Status::Mutable => Status::None,
            Status::Immutable(0) => panic!("Buffer borrow count underflow"),
            Status::Immutable(1) => Status::None,
            Status::Immutable(count) => Status::Immutable(count - 1),
        };
        self.status.set(status);
    }
}

/// A mutable, owned reference to a buffer
pub struct OwnedRef<'a, T>(&'a Buffer<'a, T>);

impl<'a, T> OwnedRef<'a, T> {
    /// Creates an immutable reference to the buffer
    pub fn reference(&self) -> SharedRef<'a, T> {
        SharedRef::new(self.0, 0..self.0.len())
    }

    /// Borrows the buffer immutably
    /// Panics if the buffer is already borrowed mutably
    pub fn borrow(&self) -> Access<'a, T> {
        Access::new(self.0, 0..self.0.len())
    }

    /// Borrows the buffer mutably
    /// Panics if the buffer is already borrowed
    pub fn borrow_mut(&self) -> AccessMut<'a, T> {
        AccessMut::new(self.0)
    }

    /// Returns the length of the buffer
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Guard struct for mutable buffer access
pub struct AccessMut<'a, T>(&'a Buffer<'a, T>);

impl<'a, T> AccessMut<'a, T> {
    fn new(buffer: &'a Buffer<'a, T>) -> Self {
        buffer.borrow(true);
        Self(buffer)
    }
}

// SAFETY: Access to the buffer is dynamically checked
impl<T> Borrow<[T]> for AccessMut<'_, T> {
    fn borrow(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.0.buffer.load(core::sync::atomic::Ordering::Acquire), self.0.len) }
    }
}

// SAFETY: Access to the buffer is dynamically checked
impl<T> BorrowMut<[T]> for AccessMut<'_, T> {
    fn borrow_mut(&mut self) -> &mut [T] {
        unsafe {
            core::slice::from_raw_parts_mut(self.0.buffer.load(core::sync::atomic::Ordering::Acquire), self.0.len)
        }
    }
}

impl<T> Drop for AccessMut<'_, T> {
    fn drop(&mut self) {
        self.0.drop_borrow();
    }
}

/// A immutable reference to a buffer
#[derive(Clone)]
pub struct SharedRef<'a, T> {
    buffer: &'a Buffer<'a, T>,
    slice: Range<usize>,
}

impl<'a, T> SharedRef<'a, T> {
    /// Creates a new immutable buffer refference
    pub fn new(buffer: &'a Buffer<'a, T>, slice: Range<usize>) -> Self {
        Self { buffer, slice }
    }

    /// Borrows the buffer immutably
    /// Panics if the buffer is already borrowed mutably
    pub fn borrow<'s>(&'s self) -> Access<'a, T> {
        Access::new(self.buffer, self.slice.clone())
    }

    /// Produces a new slice into the buffer
    pub fn slice(&self, range: Range<usize>) -> SharedRef<'a, T> {
        if range.start >= self.slice.len() || range.end > self.slice.len() {
            panic!("Slice out of bounds");
        }

        let start = self.slice.start + range.start;
        let end = start + range.len();
        SharedRef::new(self.buffer, start..end)
    }

    /// Returns the length of the buffer
    pub fn len(&self) -> usize {
        self.slice.len()
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Guard struct for immutable buffer access
pub struct Access<'a, T> {
    buffer: &'a Buffer<'a, T>,
    slice: Range<usize>,
}

impl<'a, T> Access<'a, T> {
    fn new(buffer: &'a Buffer<'a, T>, slice: Range<usize>) -> Self {
        buffer.borrow(false);
        Self { buffer, slice }
    }
}

// SAFETY: Access to the buffer is dynamically checked
impl<T> Borrow<[T]> for Access<'_, T> {
    fn borrow(&self) -> &[T] {
        let buffer = unsafe {
            core::slice::from_raw_parts(
                self.buffer.buffer.load(core::sync::atomic::Ordering::Acquire),
                self.buffer.len,
            )
        };
        &buffer[self.slice.clone()]
    }
}

impl<T> Drop for Access<'_, T> {
    fn drop(&mut self) {
        self.buffer.drop_borrow();
    }
}

/// Macro to simplify the defining a static buffer
#[macro_export]
macro_rules! define_static_buffer {
    ($name:ident, $type:ty, $contents:expr) => {
        pub(crate) mod $name {
            #![allow(dead_code)]
            #[allow(unused_imports)]
            use super::*;

            const LEN: usize = $contents.len();
            static BUFFER: ::embassy_sync::once_lock::OnceLock<$crate::buffer::Buffer<'static, $type>> =
                ::embassy_sync::once_lock::OnceLock::new();
            static mut BUFFER_STORAGE: [$type; LEN] = $contents;

            // SAFETY: The buffer is not externally visible and the constructor closure is only called once
            fn get_or_init() -> $crate::buffer::OwnedRef<'static, $type> {
                unsafe {
                    BUFFER
                        .get_or_init(|| $crate::buffer::Buffer::new(&mut *::core::ptr::addr_of_mut!(BUFFER_STORAGE)))
                        .as_owned()
                }
            }

            pub fn get_mut() -> ::core::option::Option<$crate::buffer::OwnedRef<'static, $type>> {
                if BUFFER.try_get().is_none() {
                    ::core::option::Option::Some(get_or_init())
                } else {
                    ::core::option::Option::None
                }
            }

            pub fn get() -> $crate::buffer::SharedRef<'static, $type> {
                get_or_init().reference()
            }

            pub const fn len() -> usize {
                LEN
            }
        }
    };
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;

    // Verify that only one mutable borrow is allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed mutably")]
    fn test_mut_mut_fail() {
        define_static_buffer!(buffer, u8, [0; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _mut_a = buffer.borrow_mut();
        let _mut_b = buffer.borrow_mut();
    }

    // Verify that mutable and immutable borrows are not allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed mutably")]
    fn test_mut_imm_fail() {
        define_static_buffer!(buffer, u8, [0; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _mut_a = buffer.borrow_mut();
        let _b = buffer.borrow();
    }

    // Verify that mutable and immutable borrows are not allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed immutably")]
    fn test_imm_mut_fail() {
        define_static_buffer!(buffer, u8, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _a = buffer.borrow();
        let _mut_b = buffer.borrow_mut();
    }

    // Verify that multiple immutable borrows are allowed
    #[test]
    fn test_immutable() {
        define_static_buffer!(buffer, u8, [0; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _a = buffer.borrow();
        let _b = buffer.borrow();
    }

    // Verify dropping a mutable borrow releases the buffer
    #[test]
    fn test_drop() {
        define_static_buffer!(buffer, u8, [0; 16]);
        let buffer = buffer::get_mut().unwrap();
        let mut_a = buffer.borrow_mut();
        drop(mut_a);
        let mut_b = buffer.borrow_mut();
        drop(mut_b);
        let _c = buffer.borrow();
    }

    // Test slicing
    #[test]
    fn test_slicing() {
        define_static_buffer!(buffer, u8, [0, 1, 2, 3, 4, 5, 6, 7]);
        let buffer = buffer::get_mut().unwrap();

        let slice = buffer.reference().slice(0..8);
        let sliced = slice.borrow();
        assert_eq!(sliced.borrow(), [0, 1, 2, 3, 4, 5, 6, 7]);

        let slice = buffer.reference().slice(0..4);
        let sliced = slice.borrow();
        assert_eq!(sliced.borrow(), [0, 1, 2, 3]);

        let slice = buffer.reference().slice(4..8);
        let sliced = slice.borrow();
        assert_eq!(sliced.borrow(), [4, 5, 6, 7]);

        let slice = buffer.reference().slice(4..8).slice(1..4);
        let sliced = slice.borrow();
        assert_eq!(sliced.borrow(), [5, 6, 7]);

        let slice = buffer.reference().slice(3..7);
        let sliced = slice.borrow();
        assert_eq!(sliced.borrow(), [3, 4, 5, 6]);
    }

    // Test slice starting index out of bounds
    #[test]
    #[should_panic(expected = "Slice out of bounds")]
    fn test_slice_bounds_start_fail() {
        define_static_buffer!(buffer, u8, [0, 1, 2, 3, 4, 5, 6, 7]);
        let buffer = buffer::get_mut().unwrap();

        let _slice = buffer.reference().slice(8..8);
    }

    // Test slice ending index out of bounds
    #[test]
    #[should_panic(expected = "Slice out of bounds")]
    fn test_slice_bounds_end_fail() {
        define_static_buffer!(buffer, u8, [0, 1, 2, 3, 4, 5, 6, 7]);
        let buffer = buffer::get_mut().unwrap();

        let _slice = buffer.reference().slice(0..9);
    }
}
