//! Messages originating from a PSU
use core::pin::pin;

use embassy_futures::select::select_slice;
use embedded_services::{event::Receiver, sync::Lockable};

use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu,
};

/// Data for a power policy request
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

/// Request to the power policy service
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

/// Struct used to contain PSU event receivers and manage mapping from a receiver to its corresponding device.
pub struct EventReceivers<'a, const N: usize, PSU: Lockable, R: Receiver<EventData>>
where
    PSU::Inner: psu::Psu,
{
    pub psu_devices: [&'a PSU; N],
    pub receivers: [R; N],
}

impl<'a, const N: usize, PSU: Lockable, R: Receiver<EventData>> EventReceivers<'a, N, PSU, R>
where
    PSU::Inner: psu::Psu,
{
    /// Create a new instance
    pub fn new(psu_devices: [&'a PSU; N], receivers: [R; N]) -> Self {
        Self { psu_devices, receivers }
    }

    /// Get the next pending PSU event
    pub async fn wait_event(&mut self) -> Event<'a, PSU> {
        let ((event, psu), _) = {
            let mut futures = heapless::Vec::<_, N>::new();
            for (receiver, psu) in self.receivers.iter_mut().zip(self.psu_devices.iter()) {
                // Push will never fail since the number of receivers is the same as the capacity of the vector
                let _ = futures.push(async move { (receiver.wait_next().await, psu) });
            }
            select_slice(pin!(&mut futures)).await
        };

        Event { psu, event }
    }
}
