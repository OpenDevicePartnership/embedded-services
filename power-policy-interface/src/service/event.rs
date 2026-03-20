use embedded_services::sync::Lockable;

use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu::Psu,
    service::UnconstrainedState,
};

/// Event data broadcast from the service.
///
/// This enum doesn't contain a reference to the device and is suitable
/// for receivers that don't need to know which device triggered the event
/// and allows for receivers that don't need to be generic over the device type.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventData {
    /// Consumer disconnected
    ConsumerDisconnected,
    /// Consumer connected
    ConsumerConnected(ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected,
    /// Provider connected
    ProviderConnected(ProviderPowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

impl<'device, PSU: Lockable> From<Event<'device, PSU>> for EventData
where
    PSU::Inner: Psu,
{
    fn from(value: Event<'device, PSU>) -> Self {
        match value {
            Event::ConsumerDisconnected(_) => EventData::ConsumerDisconnected,
            Event::ConsumerConnected(_, capability) => EventData::ConsumerConnected(capability),
            Event::ProviderDisconnected(_) => EventData::ProviderDisconnected,
            Event::ProviderConnected(_, capability) => EventData::ProviderConnected(capability),
            Event::Unconstrained(unconstrained) => EventData::Unconstrained(unconstrained),
        }
    }
}

/// Events broadcast from the service.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event<'device, PSU: Lockable>
where
    PSU::Inner: Psu,
{
    /// Consumer disconnected
    ConsumerDisconnected(&'device PSU),
    /// Consumer connected
    ConsumerConnected(&'device PSU, ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected(&'device PSU),
    /// Provider connected
    ProviderConnected(&'device PSU, ProviderPowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

impl<'device, PSU> Clone for Event<'device, PSU>
where
    PSU: Lockable,
    PSU::Inner: Psu,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<'device, PSU> Copy for Event<'device, PSU>
where
    PSU: Lockable,
    PSU::Inner: Psu,
{
}
