//! Traits and types for PSU notifications.

use embedded_services::sync::Lockable;

use crate::capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The requested operation would block
    WouldBlock,
}

/// PSU notifier trait
pub trait Notifier {
    /// Notify that a PSU has attached
    fn notify_attached(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Notify of updated consumer capability
    fn notify_updated_consumer_capability(
        &mut self,
        capability: Option<ConsumerPowerCapability>,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify of requested provider capability
    fn notify_requested_provider_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a PSU has disconnected
    fn notify_disconnected(&mut self, flags: ConsumerDisconnect) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a PSU has detached
    fn notify_detached(&mut self) -> impl Future<Output = Result<(), Error>>;
}

/// PSU notification handler
pub trait NotificationHandler<'device> {
    type Psu: Lockable<Inner: crate::psu::Psu> + 'device;

    /// Handle a notification that a PSU has attached
    fn process_notify_attached(&mut self, psu: &'device Self::Psu) -> impl Future<Output = Result<(), super::Error>>;
    /// Handle a notification of updated consumer capability
    fn process_notify_updated_consumer_capability(
        &mut self,
        psu: &'device Self::Psu,
        capability: Option<ConsumerPowerCapability>,
    ) -> impl Future<Output = Result<(), super::Error>>;
    /// Handle a notification of requested provider capability
    fn process_notify_requested_provider_capability(
        &mut self,
        psu: &'device Self::Psu,
        capability: Option<ProviderPowerCapability>,
    ) -> impl Future<Output = Result<(), super::Error>>;
    /// Handle a notification that a PSU has disconnected
    fn process_notify_disconnected(
        &mut self,
        psu: &'device Self::Psu,
        flags: ConsumerDisconnect,
    ) -> impl Future<Output = Result<(), super::Error>>;
    /// Handle a notification that a PSU has detached
    fn process_notify_detached(&mut self, psu: &'device Self::Psu) -> impl Future<Output = Result<(), super::Error>>;
}
