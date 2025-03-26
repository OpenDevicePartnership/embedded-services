//! Comms service message definitions

use super::GlobalPortId;

/// Message generated when a debug acessory is connected or disconnected
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DebugAccessoryMessage {
    /// Port
    pub port: GlobalPortId,
    /// Connected
    pub connected: bool,
}
