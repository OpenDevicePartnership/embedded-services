//! Messages originating from a PSU
use embedded_services::sync::Lockable;

use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu,
};

/// Data for an event broadcast from a PSU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EventData {
    /// Notify that a device has attached
    Attached,
    /// Notify that available power for consumption has changed
    UpdatedConsumerCapability(Option<ConsumerPowerCapability>),
    /// Request the given amount of power to provider
    RequestedProviderCapability(Option<ProviderPowerCapability>),
    /// Notify that a device cannot consume or provide power anymore
    Disconnected,
    /// Notify that a device has detached
    Detached,
}

/// Event broadcast from a PSU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Event<'a, D: Lockable>
where
    D::Inner: psu::Psu,
{
    /// Device that sent this request
    pub psu: &'a D,
    /// Event data
    pub event: EventData,
}
