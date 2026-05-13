//! Control types for core PD functionality

use embedded_usb_pd::{
    DataRole, PlugOrientation, PowerRole,
    pdinfo::{AltMode, PowerPathStatus},
    type_c::ConnectionState,
};

/// Port status
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatus {
    /// Current available source contract
    pub available_source_contract: Option<power_policy_interface::capability::PowerCapability>,
    /// Current available sink contract
    pub available_sink_contract: Option<power_policy_interface::capability::PowerCapability>,
    /// Current connection state
    pub connection_state: Option<ConnectionState>,
    /// Port partner supports dual-power roles
    pub dual_power: bool,
    /// plug orientation
    pub plug_orientation: PlugOrientation,
    /// power role
    pub power_role: PowerRole,
    /// data role
    pub data_role: DataRole,
    /// Active alt-modes
    pub alt_mode: AltMode,
    /// Power path status
    pub power_path: PowerPathStatus,
    /// EPR mode active
    pub epr: bool,
    /// Port partner is unconstrained
    pub unconstrained_power: bool,
}

impl PortStatus {
    /// Create a new blank port status
    /// Needed because default() is not const
    pub const fn new() -> Self {
        Self {
            available_source_contract: None,
            available_sink_contract: None,
            connection_state: None,
            dual_power: false,
            plug_orientation: PlugOrientation::CC1,
            power_role: PowerRole::Sink,
            data_role: DataRole::Dfp,
            alt_mode: AltMode::none(),
            power_path: PowerPathStatus::none(),
            epr: false,
            unconstrained_power: false,
        }
    }

    /// Check if the port is connected
    pub fn is_connected(&self) -> bool {
        matches!(
            self.connection_state,
            Some(ConnectionState::Attached)
                | Some(ConnectionState::DebugAccessory)
                | Some(ConnectionState::AudioAccessory)
        )
    }

    /// Check if a debug accessory is connected
    pub fn is_debug_accessory(&self) -> bool {
        matches!(self.connection_state, Some(ConnectionState::DebugAccessory))
    }
}

impl Default for PortStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// PD state-machine configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Default, Copy, PartialEq)]
pub struct PdStateMachineConfig {
    /// Enable or disable the PD state-machine
    pub enabled: bool,
}
