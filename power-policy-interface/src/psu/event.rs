//! Messages originating from a PSU
use embedded_services::sync::Lockable;

use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu,
};

/// Data for an event broadcast from a PSU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

/// Data for a power policy response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// The request was completed successfully
    Complete,
}

impl ResponseData {
    /// Returns an InvalidResponse error if the response is not complete
    pub fn complete_or_err(self) -> Result<(), super::Error> {
        match self {
            ResponseData::Complete => Ok(()),
        }
    }
}

/// Response from the power policy service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response {
    /// Response data
    pub data: ResponseData,
}
