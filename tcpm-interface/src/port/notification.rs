//! Traits and types for port notifications.

use embedded_services::sync::Lockable;
use embedded_usb_pd::PdError;
use embedded_usb_pd::ado::Ado;

use crate::control::dp::DpStatus;
use crate::control::pd::PortStatus;
use crate::port::event::{PortStatusEventBitfield, VdmData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The requested operation would block
    WouldBlock,
}

/// Port notifier trait
///
/// Non-blocking implementations are generally preferred, but blocking implementations are allowed
/// in-order to give the implementer more flexibility. Blocking implementations should be used with care,
/// as they can block the processing of PD events.
pub trait Notifier {
    /// Notify that a port's status has changed
    fn notify_status_changed(
        &mut self,
        status_event: PortStatusEventBitfield,
        previous_status: PortStatus,
        current_status: PortStatus,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Notify that a PD alert was received
    fn notify_alert(&mut self, alert: Ado) -> impl Future<Output = Result<(), Error>>;
    /// Notify of a VDM event
    fn notify_vdm(&mut self, vdm: VdmData) -> impl Future<Output = Result<(), Error>>;
    /// Notify that discover mode has completed
    fn notify_discover_mode_completed(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Notify of a USB mux error recovery
    fn notify_usb_mux_error_recovery(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Notify of a DisplayPort status update
    fn notify_dp_status_update(&mut self, status: DpStatus) -> impl Future<Output = Result<(), Error>>;
}

/// Port notification handler
pub trait NotificationHandler<'port> {
    type Port: Lockable<Inner: crate::port::pd::Pd> + 'port;

    /// Handle a notification that a port's status has changed
    fn process_notify_status_changed(
        &mut self,
        port: &'port Self::Port,
        status_event: PortStatusEventBitfield,
        previous_status: PortStatus,
        current_status: PortStatus,
    ) -> impl Future<Output = Result<(), PdError>>;
    /// Handle a notification that a PD alert was received
    fn process_notify_alert(
        &mut self,
        port: &'port Self::Port,
        alert: Ado,
    ) -> impl Future<Output = Result<(), PdError>>;
    /// Handle a notification of a VDM event
    fn process_notify_vdm(
        &mut self,
        port: &'port Self::Port,
        vdm: VdmData,
    ) -> impl Future<Output = Result<(), PdError>>;
    /// Handle a notification that discover mode has completed
    fn process_notify_discover_mode_completed(
        &mut self,
        port: &'port Self::Port,
    ) -> impl Future<Output = Result<(), PdError>>;
    /// Handle a notification of a USB mux error recovery
    fn process_notify_usb_mux_error_recovery(
        &mut self,
        port: &'port Self::Port,
    ) -> impl Future<Output = Result<(), PdError>>;
    /// Handle a notification of a DisplayPort status update
    fn process_notify_dp_status_update(
        &mut self,
        port: &'port Self::Port,
        status: DpStatus,
    ) -> impl Future<Output = Result<(), PdError>>;
}
