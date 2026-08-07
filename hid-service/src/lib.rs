#![no_std]
#![deprecated(
    note = "this service has been superseded by the hidi2c-target-service crate, which doesn't depend on the comms service or require 'static lifetime on devices."
)]
// This crate depends on the also-deprecated embedded-services::hid module, which is why this crate is also deprecated.
// We need to continue building even though that's deprecated.
#![allow(deprecated)]

use embedded_services::hid;

pub mod i2c;

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<B> {
    /// Error from the underlying bus
    Bus(B),
    /// HID error
    Hid(hid::Error),
    /// Error from the underlying buffer
    Buffer(embedded_services::buffer::Error),
}
