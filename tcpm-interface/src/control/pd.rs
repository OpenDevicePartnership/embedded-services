//! Control types for core PD functionality

use embedded_usb_pd::{
    DataRole, PlugOrientation, PowerRole,
    pdinfo::{AltMode, PowerPathStatus},
    pdo::{self, sink::FrsRequiredCurrent},
    type_c::ConnectionState,
};
use power_policy_interface::capability::PowerCapability;

/// Information about the negotiated source contract from PD
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PdSourceInfo {
    /// Received 5V sink PDO data from port partner
    ///
    /// Contains various flags that aren't present in other PDOs
    pub rx_fixed_5v_data: Option<pdo::sink::FixedData>,
    /// PDO associated with this contract
    pub pdo: pdo::source::Pdo,
    /// RDO associated with this contract
    pub rdo: pdo::Rdo,
}

/// Source contract
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SourceContract {
    /// Power capability
    pub capability: PowerCapability,
    /// PD contract information, if any
    pub pd: Option<PdSourceInfo>,
}

impl SourceContract {
    /// Create a source contract without PD-specific information
    pub const fn from_capability(capability: PowerCapability) -> Self {
        Self { capability, pd: None }
    }

    /// Returns true if the port partner has dual role power capability
    pub fn dual_role_power(&self) -> bool {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.dual_role_power))
            .unwrap_or(false)
    }

    /// Returns true if the port partner has higher capability
    pub fn higher_capability(&self) -> bool {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.higher_capability))
            .unwrap_or(false)
    }

    /// Returns true if the port partner has unconstrained power
    pub fn unconstrained_power(&self) -> bool {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.unconstrained_power))
            .unwrap_or(false)
    }

    /// Returns true if the port partner is USB comms capable
    pub fn usb_comms_capable(&self) -> bool {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.usb_comms_capable))
            .unwrap_or(false)
    }

    /// Returns true if the port partner has dual role data capability
    pub fn dual_role_data(&self) -> bool {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.dual_role_data))
            .unwrap_or(false)
    }

    /// Returns required FRS current for the port partner, if supported
    pub fn frs_required_current(&self) -> Option<FrsRequiredCurrent> {
        self.pd
            .and_then(|pd| pd.rx_fixed_5v_data.map(|data| data.frs_required_current))
    }
}

/// Information about the negotiated sink contract from PD
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PdSinkInfo {
    /// Received 5V source PDO data from port partner
    ///
    /// Contains various flags that aren't present in other PDOs
    pub rx_fixed_5v_data: pdo::source::FixedData,
    /// PDO associated with this contract
    pub pdo: pdo::sink::Pdo,
    /// RDO associated with this contract
    pub rdo: pdo::Rdo,
}

/// Sink contract
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SinkContract {
    /// Power capability
    pub capability: PowerCapability,
    /// PD contract information, if any
    pub pd: Option<PdSinkInfo>,
}

impl SinkContract {
    /// Create a sink contract without PD-specific information
    pub const fn from_capability(capability: PowerCapability) -> Self {
        Self { capability, pd: None }
    }

    /// Returns true if the port partner has dual role power capability
    pub fn dual_role_power(&self) -> bool {
        self.pd.map(|pd| pd.rx_fixed_5v_data.dual_role_power).unwrap_or(false)
    }

    /// Returns true if the port partner has USB suspend supported
    pub fn usb_suspend_supported(&self) -> bool {
        self.pd
            .map(|pd| pd.rx_fixed_5v_data.usb_suspend_supported)
            .unwrap_or(false)
    }

    /// Returns true if the port partner has unconstrained power
    pub fn unconstrained_power(&self) -> bool {
        self.pd
            .map(|pd| pd.rx_fixed_5v_data.unconstrained_power)
            .unwrap_or(false)
    }

    /// Returns true if the port partner is USB comms capable
    pub fn usb_comms_capable(&self) -> bool {
        self.pd.map(|pd| pd.rx_fixed_5v_data.usb_comms_capable).unwrap_or(false)
    }

    /// Returns true if the port partner has dual role data capability
    pub fn dual_role_data(&self) -> bool {
        self.pd.map(|pd| pd.rx_fixed_5v_data.dual_role_data).unwrap_or(false)
    }

    /// Returns true if the port partner has unchunked extended messages support
    pub fn unchunked_extended_messages_support(&self) -> bool {
        self.pd
            .map(|pd| pd.rx_fixed_5v_data.unchunked_extended_messages_support)
            .unwrap_or(false)
    }

    /// Returns true if the port partner is EPR capable
    pub fn epr_capable(&self) -> bool {
        self.pd.map(|pd| pd.rx_fixed_5v_data.epr_capable).unwrap_or(false)
    }
}

/// Port status
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatus {
    /// Current available source contract
    pub available_source_contract: Option<SourceContract>,
    /// Current available sink contract
    pub available_sink_contract: Option<SinkContract>,
    /// Current connection state
    pub connection_state: Option<ConnectionState>,
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
}

impl PortStatus {
    /// Create a new blank port status
    /// Needed because default() is not const
    pub const fn new() -> Self {
        Self {
            available_source_contract: None,
            available_sink_contract: None,
            connection_state: None,
            plug_orientation: PlugOrientation::CC1,
            power_role: PowerRole::Sink,
            data_role: DataRole::Dfp,
            alt_mode: AltMode::none(),
            power_path: PowerPathStatus::none(),
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
