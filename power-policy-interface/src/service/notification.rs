//! Traits and types for service notifications.
use embedded_services::sync::Lockable;

use crate::{
    capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability},
    service::UnconstrainedState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// Implementation would block
    WouldBlock,
}

/// Service notifier trait
pub trait Notifier<'device> {
    type Psu: Lockable<Inner: crate::psu::Psu> + 'device;

    /// Notify that a consumer has disconnected
    fn notify_consumer_disconnected(
        &mut self,
        psu: &'device Self::Psu,
        flags: ConsumerDisconnect,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a consumer has been connected
    fn notify_consumer_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a provider has disconnected
    fn notify_provider_disconnected(&mut self, psu: &'device Self::Psu) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a provider has connected
    fn notify_provider_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ProviderPowerCapability,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify that the unconstrained state has changed
    fn notify_unconstrained(&mut self, unconstrained: UnconstrainedState) -> impl Future<Output = Result<(), Error>>;
}
