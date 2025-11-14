//! Power policy related data structures and messages
pub mod action;
pub mod charger;
pub mod device;
pub mod flags;
pub mod policy;

pub use policy::{init, register_device};

use crate::power::policy::charger::ChargerError;

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The requested device does not exist
    InvalidDevice,
    /// The provide request was denied, contains maximum available power
    CannotProvide(Option<PowerCapability>),
    /// The consume request was denied, contains maximum available power
    CannotConsume(Option<PowerCapability>),
    /// The device is not in the correct state (expected, actual)
    InvalidState(device::StateKind, device::StateKind),
    /// Invalid response
    InvalidResponse,
    /// Busy, the device cannot respond to the request at this time
    Busy,
    /// Timeout
    Timeout,
    /// Bus error
    Bus,
    /// Charger specific error, underlying error should have more context
    Charger(ChargerError),
    /// Generic failure
    Failed,
}

/// Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Amount of power that a device can provider or consume
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
        Some(self.cmp(other))
    }
}

impl Ord for PowerCapability {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.max_power_mw().cmp(&other.max_power_mw())
    }
}

/// Power capability with consumer flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConsumerPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Consumer flags
    pub flags: flags::Consumer,
}

impl From<PowerCapability> for ConsumerPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: flags::Consumer::none(),
        }
    }
}

/// Power capability with provider flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProviderPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Provider flags
    pub flags: flags::Provider,
}

impl From<PowerCapability> for ProviderPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: flags::Provider::none(),
        }
    }
}

/// Combined power capability with flags enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerCapabilityFlags {
    /// Consumer flags
    Consumer(ConsumerPowerCapability),
    /// Provider flags
    Provider(ProviderPowerCapability),
}

/// Unconstrained state information
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UnconstrainedState {
    /// Unconstrained state
    pub unconstrained: bool,
    /// Available unconstrained devices
    pub available: usize,
}

impl UnconstrainedState {
    /// Create a new unconstrained state
    pub fn new(unconstrained: bool, available: usize) -> Self {
        Self {
            unconstrained,
            available,
        }
    }
}

/// Data to send with the comms service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommsData {
    /// Consumer disconnected
    ConsumerDisconnected(DeviceId),
    /// Consumer connected
    ConsumerConnected(DeviceId, PowerCapability),
    /// Provider disconnected
    ProviderDisconnected(DeviceId),
    /// Provider connected
    ProviderConnected(DeviceId, PowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Message to send with the comms service
pub struct CommsMessage {
    /// Message data
    pub data: CommsData,
}
