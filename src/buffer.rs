//! Efficiently passing large amounts of data between components is best done by passing references to a buffer.
//! However, async code generally requires 'static lifetimes on references. Buffers generally also need
//! to be mutable to allow data to be written to them. This module provides a way to manage ownership and access to buffers,
//! particulary those with 'static lifetimes.
//!
//! This modules provides `OwnedRef` and `SharedSlice` types. `OwnedRef` represents ownership of the underlying buffer
//! and allows mutable access to the buffer. This type does not implement `Copy` or `Clone` so as to provide compile-time
//! ownership guarantees. `SharedSlice` represents an immutable slice into the buffer. This type can be cloned
//! and can be created from an `OwnedRef`. `Access` and `AccessMut` are guard types that provide access to the buffer through
//! references tied to the lifetime of the guard struct. These types enforce Rust's aliasing and mutability rules dynamically,
//! similar to RefCell.
//!
//! This allows for some sort of producer code to own the buffer through a `OwnedRef`, and then allow access to consumers
//! through any number of `SharedSlice`.
use core::{
    borrow::{Borrow, BorrowMut},
    cell::Cell,
    marker::PhantomData,
    ops::Range,
};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Status {
    None,
    Mutable,
    Immutable(u32),
}

/// Underlying buffer storage struct
pub struct Buffer<'a> {
    buffer: *mut [u8],
    status: Cell<Status>,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Buffer<'a> {
    /// Create a new buffer from a reference
    /// SAFETY: No other code should have access to the buffer
    pub unsafe fn new(raw_buffer: &'a mut [u8]) -> Buffer<'a> {
        Buffer {
            buffer: raw_buffer,
            status: Cell::new(Status::None),
            _lifetime: PhantomData,
        }
    }

    /// Create an owned reference to the buffer
    /// SAFETY: Can be used to create mulitple mut references to the buffer
    pub unsafe fn as_owned(&'a self) -> OwnedRef<'a> {
        OwnedRef(self)
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
pub struct OwnedRef<'a>(&'a Buffer<'a>);

impl<'a> OwnedRef<'a> {
    /// Creates an immutable reference to the buffer
    pub fn reference(&self) -> SharedRef<'a> {
        SharedRef::new(self.0, 0..self.0.buffer.len())
    }

    /// Borrows the buffer immutably
    /// Panics if the buffer is already borrowed mutably
    pub fn borrow(&self) -> Access<'a> {
        Access::new(self.0, 0..self.0.buffer.len())
    }

    /// Borrows the buffer mutably
    /// Panics if the buffer is already borrowed
    pub fn borrow_mut(&self) -> AccessMut<'a> {
        AccessMut::new(self.0)
    }
}

/// Guard struct for mutable buffer access
pub struct AccessMut<'a>(&'a Buffer<'a>);

impl<'a> AccessMut<'a> {
    fn new(buffer: &'a Buffer<'a>) -> Self {
        buffer.borrow(true);
        Self(buffer)
    }
}

impl Borrow<[u8]> for AccessMut<'_> {
    fn borrow(&self) -> &[u8] {
        unsafe { &*self.0.buffer }
    }
}

impl BorrowMut<[u8]> for AccessMut<'_> {
    fn borrow_mut(&mut self) -> &mut [u8] {
        unsafe { &mut *self.0.buffer }
    }
}

impl Drop for AccessMut<'_> {
    fn drop(&mut self) {
        self.0.drop_borrow();
    }
}

/// A immutable reference to a buffer
#[derive(Clone)]
pub struct SharedRef<'a> {
    buffer: &'a Buffer<'a>,
    slice: Range<usize>,
}

impl<'a> SharedRef<'a> {
    /// Creates a new immutable buffer refference
    pub fn new(buffer: &'a Buffer<'a>, slice: Range<usize>) -> Self {
        Self { buffer, slice }
    }

    /// Borrows the buffer immutably
    /// Panics if the buffer is already borrowed mutably
    pub fn borrow<'s>(&'s self) -> Access<'a> {
        Access::new(self.buffer, self.slice.clone())
    }

    /// Produces a new slice into the buffer
    pub fn slice(&self, range: Range<usize>) -> SharedRef<'a> {
        if range.start >= self.slice.len() || range.end > self.slice.len() {
            panic!("Slice out of bounds");
        }

        let start = self.slice.start + range.start;
        let end = start + range.len();
        SharedRef::new(self.buffer, start..end)
    }
}

/// Guard struct for immutable buffer access
pub struct Access<'a> {
    buffer: &'a Buffer<'a>,
    slice: Range<usize>,
}

impl<'a> Access<'a> {
    fn new(buffer: &'a Buffer<'a>, slice: Range<usize>) -> Self {
        buffer.borrow(false);
        Self { buffer, slice }
    }
}

impl Borrow<[u8]> for Access<'_> {
    fn borrow(&self) -> &[u8] {
        let buffer = unsafe { &*self.buffer.buffer };
        &buffer[self.slice.clone()]
    }
}

impl Drop for Access<'_> {
    fn drop(&mut self) {
        self.buffer.drop_borrow();
    }
}

/// Macro to simplify the defining a static buffer
#[macro_export]
macro_rules! define_static_buffer {
    ($name:ident, $contents:expr) => {
        mod $name {
            use ::core::option::Option;
            use ::core::ptr::addr_of_mut;
            use ::embassy_sync::once_lock::OnceLock;
            use $crate::buffer::{Buffer, OwnedRef, SharedRef};

            const LEN: usize = $contents.len();
            static BUFFER: OnceLock<Buffer<'static>> = OnceLock::new();
            static mut BUFFER_STORAGE: [u8; LEN] = $contents;

            // SAFETY: The buffer is not externally visible and the constructor closure is only called once
            fn get_or_init() -> OwnedRef<'static> {
                unsafe {
                    BUFFER
                        .get_or_init(|| Buffer::new(&mut *addr_of_mut!(BUFFER_STORAGE)))
                        .as_owned()
                }
            }

            pub fn get_mut() -> Option<OwnedRef<'static>> {
                if BUFFER.try_get().is_none() {
                    Some(get_or_init())
                } else {
                    None
                }
            }

            pub fn get() -> SharedRef<'static> {
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
    use std::panic::catch_unwind;

    // Verify that only one mutable borrow is allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed mutably")]
    fn test_mut_mut_fail() {
        define_static_buffer!(buffer, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _mut_a = buffer.borrow_mut();
        let _mut_b = buffer.borrow_mut();
    }

    // Verify that mutable and immutable borrows are not allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed mutably")]
    fn test_mut_imm_fail() {
        define_static_buffer!(buffer, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _mut_a = buffer.borrow_mut();
        let _b = buffer.borrow();
    }

    // Verify that mutable and immutable borrows are not allowed
    #[test]
    #[should_panic(expected = "Buffer already borrowed immutably")]
    fn test_imm_mut_fail() {
        define_static_buffer!(buffer, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _a = buffer.borrow();
        let _mut_b = buffer.borrow_mut();
    }

    // Verify that multiple immutable borrows are allowed
    #[test]
    fn test_immutable() {
        define_static_buffer!(buffer, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let _a = buffer.borrow();
        let _b = buffer.borrow();
    }

    // Verify dropping a mutable borrow releases the buffer
    #[test]
    fn test_drop() {
        define_static_buffer!(buffer, [0u8; 16]);
        let buffer = buffer::get_mut().unwrap();
        let mut_a = buffer.borrow_mut();
        drop(mut_a);
        let mut_b = buffer.borrow_mut();
        drop(mut_b);
        let mut_c = buffer.borrow();
    }

    // Test slicing
    #[test]
    fn test_slicing() {
        define_static_buffer!(buffer, [0, 1, 2, 3, 4, 5, 6, 7]);
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
        define_static_buffer!(buffer, [0, 1, 2, 3, 4, 5, 6, 7]);
        let buffer = buffer::get_mut().unwrap();

        let slice = buffer.reference().slice(8..8);
    }

    // Test slice ending index out of bounds
    #[test]
    #[should_panic(expected = "Slice out of bounds")]
    fn test_slice_bounds_end_fail() {
        define_static_buffer!(buffer, [0, 1, 2, 3, 4, 5, 6, 7]);
        let buffer = buffer::get_mut().unwrap();

        let slice = buffer.reference().slice(0..9);
    }
}
