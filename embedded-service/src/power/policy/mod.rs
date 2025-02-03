//! Power policy related data structures and messages
pub mod device;
pub mod policy;

pub use policy::{init, register_device};

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The requested device does not exist
    InvalidDevice,
    /// The source request was denied, contains maximum available power
    CannotSource(Option<PowerCapability>),
    /// The sink request was denied, contains maximum available power
    CannotSink(Option<PowerCapability>),
    /// The device is not in the correct state
    InvalidState,
    /// Invalid response
    InvalidResponse,
    /// Bus error
    Bus,
    /// Generic failure
    Failed,
}

/// Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Amount of power that a device can source or sink
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PowerCapability {
    /// Available voltage in mV
    pub voltage_mv: u16,
    /// Max available current in mA
    pub current_ma: u16,
}

impl PowerCapability {
    /// Calculate maximum power
    pub fn max_power_mw(&self) -> u32 {
        self.voltage_mv as u32 * self.current_ma as u32 / 1000
    }
}

impl PartialOrd for PowerCapability {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.max_power_mw().cmp(&other.max_power_mw()))
    }
}

impl Ord for PowerCapability {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.max_power_mw().cmp(&other.max_power_mw())
    }
}

/// Data to send with the comms service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommsData {
    /// Sink disconnected
    SinkDisconnected(DeviceId),
    /// Sink connected
    SinkConnected(DeviceId, PowerCapability),
}

/// Message to send with the comms service
pub struct CommsMessage {
    /// Message data
    pub data: CommsData,
}
