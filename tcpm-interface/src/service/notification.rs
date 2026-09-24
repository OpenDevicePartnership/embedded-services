//! Traits and types for service notifications.

use embedded_services::sync::Lockable;
use embedded_usb_pd::GlobalPortId;

use crate::port::pd::Pd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// Implementation would block
    WouldBlock,
}

/// Service notifier trait
///
/// Non-blocking implementations are generally preferred, but blocking implementations are allowed
/// in-order to give the implementer more flexibility. Blocking implementations should be used with care,
/// as they can block the service task.
pub trait Notifier<'port> {
    type Port: Lockable<Inner: Pd> + 'port;

    /// Notify that a debug accessory was connected or disconnected
    fn notify_debug_accessory(
        &mut self,
        port: &'port Self::Port,
        connected: bool,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify of a UCSI connector change
    fn notify_ucsi_change_indicator(
        &mut self,
        port: &'port Self::Port,
        port_id: GlobalPortId,
        notify_opm: bool,
    ) -> impl Future<Output = Result<(), Error>>;
}
