use crate::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    psu::DeviceId,
    service::UnconstrainedState,
};

/// Data to send with the comms service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommsData {
    /// Consumer disconnected
    ConsumerDisconnected(DeviceId),
    /// Consumer connected
    ConsumerConnected(DeviceId, ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected(DeviceId),
    /// Provider connected
    ProviderConnected(DeviceId, ProviderPowerCapability),
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
