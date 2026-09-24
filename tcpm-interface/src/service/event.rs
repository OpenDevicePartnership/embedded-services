//! Comms service message definitions

use core::future::ready;
use core::marker::PhantomData;

use embedded_services::event::{NonBlockingSender, Sender};
use embedded_services::sync::Lockable;
use embedded_usb_pd::{GlobalPortId, ado::Ado};

use crate::{
    control::{dp::DpStatus, pd::PortStatus},
    port::{
        event::{PortStatusEventBitfield, VdmData},
        pd::Pd,
    },
    service::notification::{Error as NotificationError, Notifier},
};

/// Struct containing data for a [`PortEventData::StatusChanged`] event
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StatusChangedData {
    /// Status changed event
    pub status_event: PortStatusEventBitfield,
    /// Previous port status
    pub previous_status: PortStatus,
    /// Current port status
    pub current_status: PortStatus,
}

/// Enum to contain all port event variants
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum PortEventData {
    /// Port status change events
    StatusChanged(StatusChangedData),
    /// PD alert
    Alert(Ado),
    /// VDM
    Vdm(VdmData),
    /// Discover mode completed
    DiscoverModeCompleted,
    /// USB mux error recovery
    UsbMuxErrorRecovery,
    /// DP status update
    DpStatusUpdate(DpStatus),
}

/// Struct containing a complete port event
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortEvent<'port, Port: Lockable<Inner: Pd>> {
    pub port: &'port Port,
    pub event: PortEventData,
}

/// Message generated when a debug accessory is connected or disconnected
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DebugAccessoryData {
    /// Connected
    pub connected: bool,
}

/// UCSI connector change message
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UsciChangeIndicatorData {
    /// Port
    pub port: GlobalPortId,
    /// Notify OPM
    pub notify_opm: bool,
}

/// Top-level comms message
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EventData {
    DebugAccessory(DebugAccessoryData),
    UsciChangeIndicator(UsciChangeIndicatorData),
}

/// Top-level comms message
#[derive(Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Event<'port, Port: Lockable<Inner: Pd>> {
    pub port: &'port Port,
    pub event: EventData,
}

impl<'port, Port: Lockable<Inner: Pd>> Clone for Event<'port, Port> {
    fn clone(&self) -> Self {
        Self {
            port: self.port,
            event: self.event,
        }
    }
}

/// New-type that implements the [`Notifier`] trait for any [`NonBlockingSender<Event>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct NonBlockingSenderNotifier<'port, Port: Lockable<Inner: Pd> + 'port, S: NonBlockingSender<Event<'port, Port>>>
{
    pub sender: S,
    _phantom: PhantomData<&'port Port>,
}

impl<'port, Port: Lockable<Inner: Pd>, S: NonBlockingSender<Event<'port, Port>>>
    NonBlockingSenderNotifier<'port, Port, S>
{
    /// Create a new [`NonBlockingSenderNotifier`]
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}

impl<'port, Port: Lockable<Inner: Pd>, S: NonBlockingSender<Event<'port, Port>>> Notifier<'port>
    for NonBlockingSenderNotifier<'port, Port, S>
{
    type Port = Port;

    fn notify_debug_accessory(
        &mut self,
        port: &'port Self::Port,
        connected: bool,
    ) -> impl Future<Output = Result<(), NotificationError>> {
        ready(
            self.sender
                .try_send(Event {
                    port,
                    event: EventData::DebugAccessory(DebugAccessoryData { connected }),
                })
                .ok_or(NotificationError::WouldBlock),
        )
    }

    fn notify_ucsi_change_indicator(
        &mut self,
        port: &'port Self::Port,
        port_id: GlobalPortId,
        notify_opm: bool,
    ) -> impl Future<Output = Result<(), NotificationError>> {
        ready(
            self.sender
                .try_send(Event {
                    port,
                    event: EventData::UsciChangeIndicator(UsciChangeIndicatorData {
                        port: port_id,
                        notify_opm,
                    }),
                })
                .ok_or(NotificationError::WouldBlock),
        )
    }
}

impl<'port, Port: Lockable<Inner: Pd>, S: NonBlockingSender<Event<'port, Port>>> From<S>
    for NonBlockingSenderNotifier<'port, Port, S>
{
    fn from(sender: S) -> Self {
        Self::new(sender)
    }
}

/// New-type that implements the [`Notifier`] trait for any [`Sender<Event>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct SenderNotifier<'port, Port: Lockable<Inner: Pd> + 'port, S: Sender<Event<'port, Port>>> {
    pub sender: S,
    _phantom: PhantomData<&'port Port>,
}

impl<'port, Port: Lockable<Inner: Pd>, S: Sender<Event<'port, Port>>> SenderNotifier<'port, Port, S> {
    /// Create a new [`SenderNotifier`]
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}

impl<'port, Port: Lockable<Inner: Pd>, S: Sender<Event<'port, Port>>> Notifier<'port>
    for SenderNotifier<'port, Port, S>
{
    type Port = Port;

    async fn notify_debug_accessory(
        &mut self,
        port: &'port Self::Port,
        connected: bool,
    ) -> Result<(), NotificationError> {
        self.sender
            .send(Event {
                port,
                event: EventData::DebugAccessory(DebugAccessoryData { connected }),
            })
            .await;
        Ok(())
    }

    async fn notify_ucsi_change_indicator(
        &mut self,
        port: &'port Self::Port,
        port_id: GlobalPortId,
        notify_opm: bool,
    ) -> Result<(), NotificationError> {
        self.sender
            .send(Event {
                port,
                event: EventData::UsciChangeIndicator(UsciChangeIndicatorData {
                    port: port_id,
                    notify_opm,
                }),
            })
            .await;
        Ok(())
    }
}

impl<'port, Port: Lockable<Inner: Pd>, S: Sender<Event<'port, Port>>> From<S> for SenderNotifier<'port, Port, S> {
    fn from(sender: S) -> Self {
        Self::new(sender)
    }
}
