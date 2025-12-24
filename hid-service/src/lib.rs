#![no_std]

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
