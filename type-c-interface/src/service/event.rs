//! Comms service message definitions

use embedded_usb_pd::{GlobalPortId, ado::Ado};

use crate::port::{
    PortStatus,
    event::{PortStatusEventBitfield, VdmData},
};

/// Enum to contain all port event variants
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortEventData {
    /// Port status change events
    StatusChanged(PortStatusEventBitfield, PortStatus),
    /// PD alert
    Alert(Ado),
    /// VDM
    Vdm(VdmData),
    /// Discover mode completed
    DiscoverModeCompleted,
    /// USB mux error recovery
    UsbMuxErrorRecovery,
    /// DP status update
    DpStatusUpdate,
}

/// Struct containing a complete port event
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortEvent {
    pub port: GlobalPortId,
    pub event: PortEventData,
}

/// Message generated when a debug accessory is connected or disconnected
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DebugAccessory {
    /// Port
    pub port: GlobalPortId,
    /// Connected
    pub connected: bool,
}

/// UCSI connector change message
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UsciChangeIndicator {
    /// Port
    pub port: GlobalPortId,
    /// Notify OPM
    pub notify_opm: bool,
}

/// Top-level comms message
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    /// Debug accessory message
    DebugAccessory(DebugAccessory),
    /// UCSI CCI message
    UcsiCci(UsciChangeIndicator),
}
