//! This crate contains a service that behaves as a HID target/slave device over I2C.

#![no_std]

mod device_descriptor;
use device_descriptor::DeviceDescriptor;
pub use device_descriptor::{HardwareVersionInfo, ProductId, VendorId, VersionId};

mod attn_pin_handler;
use attn_pin_handler::AttnPinHandler;

mod error;
use error::*;

mod constrained_hid_device;
pub use constrained_hid_device::ConstrainedHidDevice;

mod service;
pub use service::{Runner, Service, TimeoutSettings};

use embedded_services::{error, info, trace, warn};

/// HID-I2C register addresses as specified in section 5.1 of the HID-I2C spec.
/// These specific values are our convention, not from the HID-I2C spec, but section 4.2 indicates
/// that all HID-I2C devices must have their own I2C bus address so there's no way to share a single
/// I2C address by leveraging different register addresses on the same I2C address.
///
#[repr(u16)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive, Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum HidI2cRegister {
    /// HID descriptor register - see section 5.1
    /// NOTE: Per the HID-I2C spec, when using ACPI for enumeration, this value needs to be put in the _DSM.
    DeviceDescriptor = 0x01,

    /// HID report descriptor register - see section 5.2
    ReportDescriptor = 0x02,

    /// Input report register - see section 6.1
    Input = 0x03,

    /// Output report register - see section 6.2
    Output = 0x04,

    /// Command register - see section 7.1.1
    Command = 0x05,

    /// Data register - see section 7.1.2
    Data = 0x06,
}
