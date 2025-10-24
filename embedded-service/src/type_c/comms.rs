//! Comms service message definitions

use embedded_usb_pd::GlobalPortId;

/// Message generated when a debug acessory is connected or disconnected
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DebugAccessoryMessage {
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
pub enum CommsMessage {
    /// Debug accessory message
    DebugAccessory(DebugAccessoryMessage),
    /// UCSI CCI message
    UcsiCci(UsciChangeIndicator),
}
