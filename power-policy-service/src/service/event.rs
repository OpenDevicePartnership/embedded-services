use embedded_services::sync::Lockable;

use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu::Psu,
    service::UnconstrainedState,
};

/// Data to send with the comms service
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event<'device, D: Lockable>
where
    D::Inner: Psu,
{
    /// Consumer disconnected
    ConsumerDisconnected(&'device D),
    /// Consumer connected
    ConsumerConnected(&'device D, ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected(&'device D),
    /// Provider connected
    ProviderConnected(&'device D, ProviderPowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

impl<'device, D> Clone for Event<'device, D>
where
    D: Lockable,
    D::Inner: Psu,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<'device, D> Copy for Event<'device, D>
where
    D: Lockable,
    D::Inner: Psu,
{
}
