use embedded_services::named::Named;
use embedded_usb_pd::vdm::structured::command::discover_identity::{sop, sop_prime};
use embedded_usb_pd::{PdError, ado::Ado};

use crate::control::{
    dp::{DpConfig, DpStatus},
    pd::{PdStateMachineConfig, PortStatus},
    svid::DiscoveredSvids,
    tbt::TbtConfig,
    usb::UsbControlConfig,
    vdm::{AttnVdm, OtherVdm, SendVdm},
};

/// Trait for basic functionality from the PD spec.
pub trait Pd: Named {
    /// Returns the port status
    fn get_port_status(&mut self) -> impl Future<Output = Result<PortStatus, PdError>>;

    /// Clear the dead battery flag for this port.
    fn clear_dead_battery_flag(&mut self) -> impl Future<Output = Result<(), PdError>>;

    /// Enable or disable sink path
    fn enable_sink_path(&mut self, enable: bool) -> impl Future<Output = Result<(), PdError>>;

    /// Get current PD alert
    fn get_pd_alert(&mut self) -> impl Future<Output = Result<Option<Ado>, PdError>>;

    /// Set port unconstrained status
    fn set_unconstrained_power(&mut self, unconstrained: bool) -> impl Future<Output = Result<(), PdError>>;

    /// Returns whether this port reports unconstrained power to the system.
    ///
    /// This is the port's own determination and can differ from
    /// [`PortStatus`], which is what the partner reports in its PDO.
    fn reports_unconstrained_power(&self) -> bool;

    /// Get the Rx Other VDM data for this port
    fn get_other_vdm(&mut self) -> impl Future<Output = Result<OtherVdm, PdError>>;
    /// Get the Rx Attention VDM data for this port
    fn get_attn_vdm(&mut self) -> impl Future<Output = Result<AttnVdm, PdError>>;
    /// Send a VDM to this port
    fn send_vdm(&mut self, tx_vdm: SendVdm) -> impl Future<Output = Result<(), PdError>>;
    /// Execute PD Data Reset for this port
    fn execute_drst(&mut self) -> impl Future<Output = Result<(), PdError>>;
    /// Execute a Hard Reset on this port.
    fn hard_reset(&mut self) -> impl Future<Output = Result<(), PdError>>;

    /// Get DisplayPort status for this port
    fn get_dp_status(&mut self) -> impl Future<Output = Result<DpStatus, PdError>>;
    /// Set DisplayPort configuration for this port
    fn set_dp_config(&mut self, config: DpConfig) -> impl Future<Output = Result<(), PdError>>;

    /// Set Thunderbolt configuration for this port
    fn set_tbt_config(&mut self, config: TbtConfig) -> impl Future<Output = Result<(), PdError>>;

    /// Set USB control configuration for this port
    fn set_usb_control(&mut self, config: UsbControlConfig) -> impl Future<Output = Result<(), PdError>>;

    /// Get this port's discovered SVIDs
    fn get_discovered_svids(&mut self) -> impl Future<Output = Result<DiscoveredSvids, PdError>>;

    /// Get the latest response from the Discover Identity command targeting SOP.
    fn get_discover_identity_sop_response(&mut self) -> impl Future<Output = Result<sop::ResponseVdos, PdError>>;

    /// Get the latest response from the Discover Identity command targeting SOP'.
    fn get_discover_identity_sop_prime_response(
        &mut self,
    ) -> impl Future<Output = Result<sop_prime::ResponseVdos, PdError>>;
}

/// PD state machine related controller functionality
pub trait StateMachine: Pd {
    /// Set PD state-machine configuration for this port
    fn set_pd_state_machine_config(
        &mut self,
        config: PdStateMachineConfig,
    ) -> impl Future<Output = Result<(), PdError>>;
}
