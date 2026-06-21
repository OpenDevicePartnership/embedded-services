use std::num::NonZeroU8;

use embassy_sync::{channel, mutex::Mutex, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::named::Named;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::vdm::structured::command::discover_identity::{sop, sop_prime};
use embedded_usb_pd::{LocalPortId, PdError};
use embedded_usb_pd::{PowerRole, type_c::Current};
use embedded_usb_pd::{type_c::ConnectionState, ucsi::lpm};
use log::{debug, info};

use power_policy_interface::capability::PowerCapability;
use type_c_interface::control::dp::{DpConfig, DpPinConfig, DpStatus};
use type_c_interface::control::pd::{PdStateMachineConfig, PortStatus};
use type_c_interface::control::power::SystemPowerState;
use type_c_interface::control::retimer::RetimerFwUpdateState;
use type_c_interface::control::svid::DiscoveredSvids;
use type_c_interface::control::tbt::TbtConfig;
use type_c_interface::control::type_c::TypeCStateMachineState;
use type_c_interface::control::usb::UsbControlConfig;
use type_c_interface::control::vdm::{AttnVdm, OtherVdm, SendVdm};
use type_c_interface::port::event::PortEventBitfield;
use type_c_interface::util::power_capability_from_current;
use type_c_service::controller::state::SharedState;

pub struct ControllerState {
    events: Signal<GlobalRawMutex, PortEventBitfield>,
    status: Mutex<GlobalRawMutex, PortStatus>,
    pd_alert: Mutex<GlobalRawMutex, Option<Ado>>,
}

impl ControllerState {
    pub const fn new() -> Self {
        Self {
            events: Signal::new(),
            status: Mutex::new(PortStatus::new()),
            pd_alert: Mutex::new(None),
        }
    }

    pub fn create_interrupt_receiver(&self) -> InterruptReceiver<'_> {
        InterruptReceiver { events: &self.events }
    }

    /// Simulate a connection
    pub async fn connect(&self, role: PowerRole, capability: PowerCapability, debug: bool, unconstrained: bool) {
        let mut status = PortStatus::new();
        status.connection_state = Some(if debug {
            ConnectionState::DebugAccessory
        } else {
            ConnectionState::Attached
        });

        let mut events = PortEventBitfield::none();
        match role {
            PowerRole::Source => {
                status.available_source_contract = Some(capability);
                status.unconstrained_power = unconstrained;
                events.status.set_new_power_contract_as_provider(true);
            }
            PowerRole::Sink => {
                status.available_sink_contract = Some(capability);
                status.unconstrained_power = unconstrained;
                events.status.set_new_power_contract_as_consumer(true);
                events.status.set_sink_ready(true);
            }
        }
        *self.status.lock().await = status;

        events.status.set_plug_inserted_or_removed(true);
        self.events.signal(events);
    }

    /// Simulate a sink connecting
    pub async fn connect_sink(&self, capability: PowerCapability, unconstrained: bool) {
        self.connect(PowerRole::Sink, capability, false, unconstrained).await;
    }

    /// Simulate a disconnection
    pub async fn disconnect(&self) {
        *self.status.lock().await = PortStatus::default();

        let mut events = PortEventBitfield::none();
        events.status.set_plug_inserted_or_removed(true);
        self.events.signal(events);
    }

    /// Simulate a debug accessory source connecting
    pub async fn connect_debug_accessory_source(&self, current: Current) {
        self.connect(PowerRole::Source, power_capability_from_current(current), true, false)
            .await;
    }

    /// Simulate a PD alert
    pub async fn send_pd_alert(&self, ado: Ado) {
        *self.pd_alert.lock().await = Some(ado);

        let mut events = PortEventBitfield::none();
        events.notification.set_alert(true);
        self.events.signal(events);
    }
}

impl Default for ControllerState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Controller<'a> {
    state: &'a ControllerState,
    name: &'static str,
}

impl<'a> Controller<'a> {
    pub fn new(state: &'a ControllerState, name: &'static str) -> Self {
        Self { state, name }
    }

    /// Function to demonstrate calling functions directly on the controller
    pub fn custom_function(&mut self) {
        info!("Custom function called on controller");
    }
}

pub struct InterruptReceiver<'a> {
    events: &'a Signal<GlobalRawMutex, PortEventBitfield>,
}

impl<const N: usize> type_c_service::controller::event_receiver::InterruptReceiver<N> for InterruptReceiver<'_> {
    async fn wait_interrupt(&mut self) -> [PortEventBitfield; N] {
        let events = self.events.wait().await;
        let mut result = [PortEventBitfield::none(); N];
        result[0] = events;
        result
    }
}

impl Named for Controller<'_> {
    fn name(&self) -> &'static str {
        self.name
    }
}

impl type_c_interface::controller::Controller for Controller<'_> {
    async fn reset_controller(&mut self) -> Result<(), PdError> {
        debug!("Reset controller");
        Ok(())
    }
}

impl type_c_interface::controller::pd::Pd for Controller<'_> {
    async fn get_port_status(&mut self, _port: LocalPortId) -> Result<PortStatus, PdError> {
        debug!("Get port status: {:#?}", *self.state.status.lock().await);
        Ok(*self.state.status.lock().await)
    }

    async fn enable_sink_path(&mut self, _port: LocalPortId, enable: bool) -> Result<(), PdError> {
        debug!("Enable sink path: {enable}");
        Ok(())
    }

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, PdError> {
        let pd_alert = self.state.pd_alert.lock().await;
        if let Some(ado) = *pd_alert {
            debug!("Port{}: Get PD alert: {ado:#?}", port.0);
            Ok(Some(ado))
        } else {
            debug!("Port{}: No PD alert", port.0);
            Ok(None)
        }
    }

    async fn set_unconstrained_power(&mut self, _port: LocalPortId, unconstrained: bool) -> Result<(), PdError> {
        debug!("Set unconstrained power: {unconstrained}");
        Ok(())
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), PdError> {
        debug!("clear_dead_battery_flag(port: {port:?})");
        Ok(())
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<OtherVdm, PdError> {
        debug!("Get other VDM for port {port:?}");
        Ok(OtherVdm::default())
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<AttnVdm, PdError> {
        debug!("Get attention VDM for port {port:?}");
        Ok(AttnVdm::default())
    }

    async fn send_vdm(&mut self, port: LocalPortId, tx_vdm: SendVdm) -> Result<(), PdError> {
        debug!("Send VDM for port {port:?}: {tx_vdm:?}");
        Ok(())
    }

    async fn set_usb_control(&mut self, port: LocalPortId, config: UsbControlConfig) -> Result<(), PdError> {
        debug!(
            "set_usb_control(port: {port:?}, usb2: {}, usb3: {}, usb4: {})",
            config.usb2_enabled, config.usb3_enabled, config.usb4_enabled
        );
        Ok(())
    }

    async fn get_dp_status(&mut self, port: LocalPortId) -> Result<DpStatus, PdError> {
        debug!("Get DisplayPort status for port {port:?}");
        Ok(DpStatus {
            alt_mode_entered: false,
            dfp_d_pin_cfg: DpPinConfig::default(),
        })
    }

    async fn set_dp_config(&mut self, port: LocalPortId, config: DpConfig) -> Result<(), PdError> {
        debug!(
            "Set DisplayPort config for port {port:?}: enable={}, pin_cfg={:?}",
            config.enable, config.dfp_d_pin_cfg
        );
        Ok(())
    }

    async fn execute_drst(&mut self, port: LocalPortId) -> Result<(), PdError> {
        debug!("Execute PD Data Reset for port {port:?}");
        Ok(())
    }

    async fn set_tbt_config(&mut self, port: LocalPortId, config: TbtConfig) -> Result<(), PdError> {
        debug!("Set Thunderbolt config for port {port:?}: {config:?}");
        Ok(())
    }

    async fn hard_reset(&mut self, port: LocalPortId) -> Result<(), PdError> {
        debug!("Hard reset for port {port:?}");
        Ok(())
    }

    async fn get_discovered_svids(&mut self, port: LocalPortId) -> Result<DiscoveredSvids, PdError> {
        debug!("Get discovered SVIDs for port {port:?}");
        Ok(DiscoveredSvids::default())
    }

    async fn get_discover_identity_sop_response(&mut self, port: LocalPortId) -> Result<sop::ResponseVdos, PdError> {
        debug!("Get Discover Identity SOP response for port {port:?}");
        Err(PdError::Failed)
    }

    async fn get_discover_identity_sop_prime_response(
        &mut self,
        port: LocalPortId,
    ) -> Result<sop_prime::ResponseVdos, PdError> {
        debug!("Get Discover Identity SOP' response for port {port:?}");
        Err(PdError::Failed)
    }
}

impl type_c_interface::controller::max_sink_voltage::MaxSinkVoltage for Controller<'_> {
    async fn set_max_sink_voltage(&mut self, port: LocalPortId, voltage_mv: Option<u16>) -> Result<(), PdError> {
        debug!("Set max sink voltage for port {}: {:?}", port.0, voltage_mv);
        Ok(())
    }
}

impl type_c_interface::controller::pd::StateMachine for Controller<'_> {
    async fn set_pd_state_machine_config(
        &mut self,
        port: LocalPortId,
        config: PdStateMachineConfig,
    ) -> Result<(), PdError> {
        debug!("Set PD State Machine config for port {port:?}: {config:?}");
        Ok(())
    }
}

impl type_c_interface::controller::type_c::StateMachine for Controller<'_> {
    async fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: TypeCStateMachineState,
    ) -> Result<(), PdError> {
        debug!("Set Type-C State Machine state for port {port:?}: {state:?}");
        Ok(())
    }
}

impl type_c_interface::ucsi::Lpm for Controller<'_> {
    async fn execute_lpm_command(&mut self, command: lpm::LocalCommand) -> Result<Option<lpm::ResponseData>, PdError> {
        debug!("Execute UCSI command for port {:?}: {command:?}", command.port());
        match command.operation() {
            lpm::CommandData::GetConnectorStatus => Ok(Some(lpm::ResponseData::GetConnectorStatus(
                lpm::get_connector_status::ResponseData::default(),
            ))),
            _ => Err(PdError::UnrecognizedCommand),
        }
    }
}

impl type_c_interface::controller::electrical_disconnect::ElectricalDisconnect for Controller<'_> {
    async fn execute_electrical_disconnect(
        &mut self,
        port: LocalPortId,
        reconnect_time_s: Option<NonZeroU8>,
    ) -> Result<(), PdError> {
        debug!("Execute electrical disconnect for port {port:?} with reconnect time {reconnect_time_s:?}");
        Ok(())
    }
}

impl type_c_interface::controller::power::SystemPowerStateStatus for Controller<'_> {
    async fn set_system_power_state_status(
        &mut self,
        port: LocalPortId,
        state: SystemPowerState,
    ) -> Result<(), PdError> {
        debug!("Set system power state for port {port:?}: {state:?}");
        Ok(())
    }
}

impl type_c_interface::controller::retimer::Retimer for Controller<'_> {
    async fn get_rt_fw_update_status(&mut self, _port: LocalPortId) -> Result<RetimerFwUpdateState, PdError> {
        debug!("Get retimer fw update status");
        Ok(RetimerFwUpdateState::Inactive)
    }

    async fn set_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), PdError> {
        debug!("Set retimer fw update state");
        Ok(())
    }

    async fn clear_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), PdError> {
        debug!("Clear retimer fw update state");
        Ok(())
    }

    async fn set_rt_compliance(&mut self, _port: LocalPortId) -> Result<(), PdError> {
        debug!("Set retimer compliance");
        Ok(())
    }

    async fn reconfigure_retimer(&mut self, port: LocalPortId) -> Result<(), PdError> {
        debug!("reconfigure_retimer(port: {port:?})");
        Ok(())
    }
}

pub type Port<'a> = type_c_service::controller::Port<
    'a,
    Mutex<GlobalRawMutex, Controller<'a>>,
    Mutex<GlobalRawMutex, SharedState>,
    channel::DynamicSender<'a, type_c_interface::service::event::PortEventData>,
    channel::DynamicSender<'a, power_policy_interface::psu::event::EventData>,
    channel::DynamicSender<'a, type_c_service::controller::event::Loopback>,
>;
