#![allow(dead_code)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use core::num::NonZeroU8;
use embassy_sync::{mutex::Mutex, signal::Signal};
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOfferResponse, HostToken};
use embedded_services::{
    GlobalRawMutex,
    power::policy::PowerCapability,
    type_c::{
        controller::{
            AttnVdm, ControllerStatus, DiscoveredSvids, DpConfig, DpStatus, OtherVdm, PdStateMachineConfig, PortStatus,
            RetimerFwUpdateState, SendVdm, SystemPowerState, TbtConfig, TypeCStateMachineState, UsbControlConfig,
        },
        event::PortEvent,
    },
};
use embedded_usb_pd::{Error, ado::Ado};
use embedded_usb_pd::{LocalPortId, PdError};
use embedded_usb_pd::{PowerRole, type_c::Current};
use embedded_usb_pd::{type_c::ConnectionState, ucsi::lpm};
use log::{debug, info};
use std::collections::VecDeque;

/// Enum containing all possible function calls
#[derive(Debug, PartialEq, Eq)]
pub enum FnCall {
    WaitPortEvent,
    ClearPortEvents(LocalPortId),
    GetPortStatus(LocalPortId),
    EnableSinkPath(LocalPortId, bool),
    GetControllerStatus,
    ResetController,
    GetRtFwUpdateStatus(LocalPortId),
    SetRtFwUpdateState(LocalPortId),
    ClearRtFwUpdateState(LocalPortId),
    SetRtCompliance(LocalPortId),
    GetPdAlert(LocalPortId),
    SetUnconstrainedPower(LocalPortId, bool),
    GetActiveFwVersion,
    StartFwUpdate,
    AbortFwUpdate,
    FinalizeFwUpdate,
    WriteFwContents(usize, Vec<u8>),
    SetMaxSinkVoltage(LocalPortId, Option<u16>),
    ReconfigureRetimer(LocalPortId),
    ClearDeadBatteryFlag(LocalPortId),
    GetOtherVdm(LocalPortId),
    GetAttnVdm(LocalPortId),
    SendVdm(LocalPortId, SendVdm),
    SetUsbControl(LocalPortId, UsbControlConfig),
    GetDpStatus(LocalPortId),
    SetDpConfig(LocalPortId, DpConfig),
    ExecuteDrst(LocalPortId),
    SetTbtConfig(LocalPortId, TbtConfig),
    SetPdStateMachineConfig(LocalPortId, PdStateMachineConfig),
    SetTypeCStateMachineConfig(LocalPortId, TypeCStateMachineState),
    ExecuteUcsiCommand(lpm::LocalCommand),
    ExecuteElectricalDisconnect(LocalPortId, Option<NonZeroU8>),
    SetPowerState(LocalPortId, SystemPowerState),
    GetDiscoveredSvids(LocalPortId),
    HardReset(LocalPortId),
    GetDiscoverIdentitySopResponse(LocalPortId),
    GetDiscoverIdentitySopPrimeResponse(LocalPortId),
}

pub struct ControllerState<'a> {
    /// The function calls that have been made
    pub fn_calls: VecDeque<FnCall>,
    /// Next result to return for [`Controller::wait_port_event`]
    pub next_result_wait_port_event: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::clear_port_events`]
    pub next_result_clear_port_events: VecDeque<Result<PortEvent, PdError>>,
    /// Next result to return for [`Controller::get_port_status`]
    pub next_result_get_port_status: VecDeque<Result<PortStatus, PdError>>,
    /// Next result to return for [`Controller::enable_sink_path`]
    pub next_result_enable_sink_path: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_controller_status`]
    pub next_result_get_controller_status: VecDeque<Result<ControllerStatus<'static>, PdError>>,
    /// Next result to return for [`Controller::reset_controller`]
    pub next_result_reset_controller: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_rt_fw_update_status`]
    pub next_result_get_rt_fw_update_status: VecDeque<Result<RetimerFwUpdateState, PdError>>,
    /// Next result to return for [`Controller::set_rt_fw_update_state`]
    pub next_result_set_rt_fw_update_state: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::clear_rt_fw_update_state`]
    pub next_result_clear_rt_fw_update_state: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_rt_compliance`]
    pub next_result_set_rt_compliance: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_pd_alert`]
    pub next_result_get_pd_alert: VecDeque<Result<Option<Ado>, PdError>>,
    /// Next result to return for [`Controller::set_unconstrained_power`]
    pub next_result_set_unconstrained_power: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_active_fw_version`]
    pub next_result_get_active_fw_version: VecDeque<Result<u32, PdError>>,
    /// Next result to return for [`Controller::start_fw_update`]
    pub next_result_start_fw_update: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::abort_fw_update`]
    pub next_result_abort_fw_update: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::finalize_fw_update`]
    pub next_result_finalize_fw_update: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::write_fw_contents`]
    pub next_result_write_fw_contents: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_max_sink_voltage`]
    pub next_result_set_max_sink_voltage: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::reconfigure_retimer`]
    pub next_result_reconfigure_retimer: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::clear_dead_battery_flag`]
    pub next_result_clear_dead_battery_flag: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_other_vdm`]
    pub next_result_get_other_vdm: VecDeque<Result<OtherVdm, PdError>>,
    /// Next result to return for [`Controller::get_attn_vdm`]
    pub next_result_get_attn_vdm: VecDeque<Result<AttnVdm, PdError>>,
    /// Next result to return for [`Controller::send_vdm`]
    pub next_result_send_vdm: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_usb_control`]
    pub next_result_set_usb_control: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_dp_status`]
    pub next_result_get_dp_status: VecDeque<Result<DpStatus, PdError>>,
    /// Next result to return for [`Controller::set_dp_config`]
    pub next_result_set_dp_config: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::execute_drst`]
    pub next_result_execute_drst: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_tbt_config`]
    pub next_result_set_tbt_config: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_pd_state_machine_config`]
    pub next_result_set_pd_state_machine_config: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_type_c_state_machine_config`]
    pub next_result_set_type_c_state_machine_config: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::execute_ucsi_command`]
    pub next_result_execute_ucsi_command: VecDeque<Result<Option<lpm::ResponseData>, PdError>>,
    /// Next result to return for [`Controller::execute_electrical_disconnect`]
    pub next_result_execute_electrical_disconnect: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::set_power_state`]
    pub next_result_set_power_state: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_discovered_svids`]
    pub next_result_get_discovered_svids: VecDeque<Result<DiscoveredSvids, PdError>>,
    /// Next result to return for [`Controller::hard_reset`]
    pub next_result_hard_reset: VecDeque<Result<(), PdError>>,
    /// Next result to return for [`Controller::get_discover_identity_sop_response`]
    pub next_result_get_discover_identity_sop_response:
        VecDeque<Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop::ResponseVdos, PdError>>,
    /// Next result to return for [`Controller::get_discover_identity_sop_prime_response`]
    pub next_result_get_discover_identity_sop_prime_response: VecDeque<
        Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop_prime::ResponseVdos, PdError>,
    >,
    interrupt: &'a Signal<GlobalRawMutex, ()>,
}

impl<'a> ControllerState<'a> {
    pub fn new(interrupt: &'a Signal<GlobalRawMutex, ()>) -> Self {
        Self {
            fn_calls: VecDeque::new(),
            next_result_wait_port_event: VecDeque::new(),
            next_result_clear_port_events: VecDeque::new(),
            next_result_get_port_status: VecDeque::new(),
            next_result_enable_sink_path: VecDeque::new(),
            next_result_get_controller_status: VecDeque::new(),
            next_result_reset_controller: VecDeque::new(),
            next_result_get_rt_fw_update_status: VecDeque::new(),
            next_result_set_rt_fw_update_state: VecDeque::new(),
            next_result_clear_rt_fw_update_state: VecDeque::new(),
            next_result_set_rt_compliance: VecDeque::new(),
            next_result_get_pd_alert: VecDeque::new(),
            next_result_set_unconstrained_power: VecDeque::new(),
            next_result_get_active_fw_version: VecDeque::new(),
            next_result_start_fw_update: VecDeque::new(),
            next_result_abort_fw_update: VecDeque::new(),
            next_result_finalize_fw_update: VecDeque::new(),
            next_result_write_fw_contents: VecDeque::new(),
            next_result_set_max_sink_voltage: VecDeque::new(),
            next_result_reconfigure_retimer: VecDeque::new(),
            next_result_clear_dead_battery_flag: VecDeque::new(),
            next_result_get_other_vdm: VecDeque::new(),
            next_result_get_attn_vdm: VecDeque::new(),
            next_result_send_vdm: VecDeque::new(),
            next_result_set_usb_control: VecDeque::new(),
            next_result_get_dp_status: VecDeque::new(),
            next_result_set_dp_config: VecDeque::new(),
            next_result_execute_drst: VecDeque::new(),
            next_result_set_tbt_config: VecDeque::new(),
            next_result_set_pd_state_machine_config: VecDeque::new(),
            next_result_set_type_c_state_machine_config: VecDeque::new(),
            next_result_execute_ucsi_command: VecDeque::new(),
            next_result_execute_electrical_disconnect: VecDeque::new(),
            next_result_set_power_state: VecDeque::new(),
            next_result_get_discovered_svids: VecDeque::new(),
            next_result_hard_reset: VecDeque::new(),
            next_result_get_discover_identity_sop_response: VecDeque::new(),
            next_result_get_discover_identity_sop_prime_response: VecDeque::new(),
            interrupt,
        }
    }

    /// Simulate a connection
    pub async fn connect(&mut self, role: PowerRole, capability: PowerCapability, debug: bool, unconstrained: bool) {
        let mut status = PortStatus::new();
        status.connection_state = Some(if debug {
            ConnectionState::DebugAccessory
        } else {
            ConnectionState::Attached
        });
        match role {
            PowerRole::Source => {
                status.available_source_contract = Some(capability);
                status.unconstrained_power = unconstrained;
            }
            PowerRole::Sink => {
                status.available_sink_contract = Some(capability);
                status.unconstrained_power = unconstrained;
            }
        }
        self.next_result_get_port_status.push_back(Ok(status));

        let mut events = PortEvent::none();
        events.status.set_plug_inserted_or_removed(true);
        events.status.set_new_power_contract_as_consumer(true);
        events.status.set_sink_ready(true);
        self.next_result_clear_port_events.push_back(Ok(events));
        self.next_result_wait_port_event.push_back(Ok(()));
        self.interrupt.signal(());
    }

    /// Simulate a sink connecting
    pub async fn connect_sink(&mut self, capability: PowerCapability, unconstrained: bool) {
        self.connect(PowerRole::Sink, capability, false, unconstrained).await;
    }

    /// Simulate a disconnection
    pub async fn disconnect(&mut self) {
        self.next_result_get_port_status.push_back(Ok(PortStatus::default()));

        let mut events = PortEvent::none();
        events.status.set_plug_inserted_or_removed(true);
        self.next_result_clear_port_events.push_back(Ok(events));
        self.next_result_wait_port_event.push_back(Ok(()));
        self.interrupt.signal(());
    }

    /// Simulate a debug accessory source connecting
    pub async fn connect_debug_accessory_source(&mut self, current: Current) {
        self.connect(PowerRole::Source, current.into(), true, false).await;
    }

    /// Simulate a PD alert
    pub async fn send_pd_alert(&mut self, ado: Ado) {
        self.next_result_get_pd_alert.push_back(Ok(Some(ado)));

        let mut events = PortEvent::none();
        events.notification.set_alert(true);
        self.next_result_clear_port_events.push_back(Ok(events));
        self.next_result_wait_port_event.push_back(Ok(()));
        self.interrupt.signal(());
    }
}

pub struct Controller<'a> {
    state: &'a Mutex<GlobalRawMutex, ControllerState<'a>>,
    interrupt: &'a Signal<GlobalRawMutex, ()>,
}

impl<'a> Controller<'a> {
    pub fn new(
        state: &'a Mutex<GlobalRawMutex, ControllerState<'a>>,
        interrupt: &'a Signal<GlobalRawMutex, ()>,
    ) -> Self {
        Self { state, interrupt }
    }

    /// Function to demonstrate calling functions directly on the controller
    pub fn custom_function(&self) {
        info!("Custom function called on controller");
    }
}

impl embedded_services::type_c::controller::Controller for Controller<'_> {
    type BusError = ();

    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        self.interrupt.wait().await;
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::WaitPortEvent);
        state
            .next_result_wait_port_event
            .pop_front()
            .expect("next_result_wait_port_event not set")
            .map_err(Error::Pd)
    }

    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEvent, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ClearPortEvents(port));
        let result = state
            .next_result_clear_port_events
            .pop_front()
            .expect("next_result_clear_port_events not set");
        debug!("Clear port events: {result:#?}");
        result.map_err(Error::Pd)
    }

    async fn get_port_status(&mut self, port: LocalPortId) -> Result<PortStatus, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetPortStatus(port));
        let result = state
            .next_result_get_port_status
            .pop_front()
            .expect("next_result_get_port_status not set");
        debug!("Get port status: {result:#?}");
        result.map_err(Error::Pd)
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::EnableSinkPath(port, enable));
        debug!("Enable sink path: {enable}");
        state
            .next_result_enable_sink_path
            .pop_front()
            .expect("next_result_enable_sink_path not set")
            .map_err(Error::Pd)
    }

    async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetControllerStatus);
        debug!("Get controller status");
        state
            .next_result_get_controller_status
            .pop_front()
            .expect("next_result_get_controller_status not set")
            .map_err(Error::Pd)
    }

    async fn reset_controller(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ResetController);
        debug!("Reset controller");
        state
            .next_result_reset_controller
            .pop_front()
            .expect("next_result_reset_controller not set")
            .map_err(Error::Pd)
    }

    async fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<RetimerFwUpdateState, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetRtFwUpdateStatus(port));
        debug!("Get retimer fw update status");
        state
            .next_result_get_rt_fw_update_status
            .pop_front()
            .expect("next_result_get_rt_fw_update_status not set")
            .map_err(Error::Pd)
    }

    async fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::SetRtFwUpdateState(port));
        debug!("Set retimer fw update state");
        state
            .next_result_set_rt_fw_update_state
            .pop_front()
            .expect("next_result_set_rt_fw_update_state not set")
            .map_err(Error::Pd)
    }

    async fn clear_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ClearRtFwUpdateState(port));
        debug!("Clear retimer fw update state");
        state
            .next_result_clear_rt_fw_update_state
            .pop_front()
            .expect("next_result_clear_rt_fw_update_state not set")
            .map_err(Error::Pd)
    }

    async fn set_rt_compliance(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::SetRtCompliance(port));
        debug!("Set retimer compliance");
        state
            .next_result_set_rt_compliance
            .pop_front()
            .expect("next_result_set_rt_compliance not set")
            .map_err(Error::Pd)
    }

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetPdAlert(port));
        let result = state
            .next_result_get_pd_alert
            .pop_front()
            .expect("next_result_get_pd_alert not set");
        if let Ok(Some(ado)) = &result {
            debug!("Port{}: Get PD alert: {ado:#?}", port.0);
        } else {
            debug!("Port{}: No PD alert", port.0);
        }
        result.map_err(Error::Pd)
    }

    async fn set_unconstrained_power(
        &mut self,
        port: LocalPortId,
        unconstrained: bool,
    ) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state
            .fn_calls
            .push_back(FnCall::SetUnconstrainedPower(port, unconstrained));
        debug!("Set unconstrained power: {unconstrained}");
        state
            .next_result_set_unconstrained_power
            .pop_front()
            .expect("next_result_set_unconstrained_power not set")
            .map_err(Error::Pd)
    }

    async fn get_active_fw_version(&mut self) -> Result<u32, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetActiveFwVersion);
        state
            .next_result_get_active_fw_version
            .pop_front()
            .expect("next_result_get_active_fw_version not set")
            .map_err(Error::Pd)
    }

    async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::StartFwUpdate);
        state
            .next_result_start_fw_update
            .pop_front()
            .expect("next_result_start_fw_update not set")
            .map_err(Error::Pd)
    }

    async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::AbortFwUpdate);
        state
            .next_result_abort_fw_update
            .pop_front()
            .expect("next_result_abort_fw_update not set")
            .map_err(Error::Pd)
    }

    async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::FinalizeFwUpdate);
        state
            .next_result_finalize_fw_update
            .pop_front()
            .expect("next_result_finalize_fw_update not set")
            .map_err(Error::Pd)
    }

    async fn write_fw_contents(&mut self, offset: usize, data: &[u8]) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::WriteFwContents(offset, data.to_vec()));
        state
            .next_result_write_fw_contents
            .pop_front()
            .expect("next_result_write_fw_contents not set")
            .map_err(Error::Pd)
    }

    async fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::SetMaxSinkVoltage(port, voltage_mv));
        debug!("Set max sink voltage for port {}: {:?}", port.0, voltage_mv);
        state
            .next_result_set_max_sink_voltage
            .pop_front()
            .expect("next_result_set_max_sink_voltage not set")
            .map_err(Error::Pd)
    }

    async fn reconfigure_retimer(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ReconfigureRetimer(port));
        debug!("reconfigure_retimer(port: {port:?})");
        state
            .next_result_reconfigure_retimer
            .pop_front()
            .expect("next_result_reconfigure_retimer not set")
            .map_err(Error::Pd)
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ClearDeadBatteryFlag(port));
        debug!("clear_dead_battery_flag(port: {port:?})");
        state
            .next_result_clear_dead_battery_flag
            .pop_front()
            .expect("next_result_clear_dead_battery_flag not set")
            .map_err(Error::Pd)
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<OtherVdm, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetOtherVdm(port));
        debug!("Get other VDM for port {port:?}");
        state
            .next_result_get_other_vdm
            .pop_front()
            .expect("next_result_get_other_vdm not set")
            .map_err(Error::Pd)
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<AttnVdm, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetAttnVdm(port));
        debug!("Get attention VDM for port {port:?}");
        state
            .next_result_get_attn_vdm
            .pop_front()
            .expect("next_result_get_attn_vdm not set")
            .map_err(Error::Pd)
    }

    async fn send_vdm(&mut self, port: LocalPortId, tx_vdm: SendVdm) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!("Send VDM for port {port:?}: {tx_vdm:?}");
        state.fn_calls.push_back(FnCall::SendVdm(port, tx_vdm));
        state
            .next_result_send_vdm
            .pop_front()
            .expect("next_result_send_vdm not set")
            .map_err(Error::Pd)
    }

    async fn set_usb_control(
        &mut self,
        port: LocalPortId,
        config: UsbControlConfig,
    ) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!(
            "set_usb_control(port: {port:?}, usb2: {}, usb3: {}, usb4: {})",
            config.usb2_enabled, config.usb3_enabled, config.usb4_enabled
        );
        state.fn_calls.push_back(FnCall::SetUsbControl(port, config));
        state
            .next_result_set_usb_control
            .pop_front()
            .expect("next_result_set_usb_control not set")
            .map_err(Error::Pd)
    }

    async fn get_dp_status(&mut self, port: LocalPortId) -> Result<DpStatus, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetDpStatus(port));
        debug!("Get DisplayPort status for port {port:?}");
        state
            .next_result_get_dp_status
            .pop_front()
            .expect("next_result_get_dp_status not set")
            .map_err(Error::Pd)
    }

    async fn set_dp_config(&mut self, port: LocalPortId, config: DpConfig) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!(
            "Set DisplayPort config for port {port:?}: enable={}, pin_cfg={:?}",
            config.enable, config.dfp_d_pin_cfg
        );
        state.fn_calls.push_back(FnCall::SetDpConfig(port, config));
        state
            .next_result_set_dp_config
            .pop_front()
            .expect("next_result_set_dp_config not set")
            .map_err(Error::Pd)
    }

    async fn execute_drst(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::ExecuteDrst(port));
        debug!("Execute PD Data Reset for port {port:?}");
        state
            .next_result_execute_drst
            .pop_front()
            .expect("next_result_execute_drst not set")
            .map_err(Error::Pd)
    }

    async fn set_tbt_config(&mut self, port: LocalPortId, config: TbtConfig) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!("Set Thunderbolt config for port {port:?}: {config:?}");
        state.fn_calls.push_back(FnCall::SetTbtConfig(port, config));
        state
            .next_result_set_tbt_config
            .pop_front()
            .expect("next_result_set_tbt_config not set")
            .map_err(Error::Pd)
    }

    async fn set_pd_state_machine_config(
        &mut self,
        port: LocalPortId,
        config: PdStateMachineConfig,
    ) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!("Set PD State Machine config for port {port:?}: {config:?}");
        state.fn_calls.push_back(FnCall::SetPdStateMachineConfig(port, config));
        state
            .next_result_set_pd_state_machine_config
            .pop_front()
            .expect("next_result_set_pd_state_machine_config not set")
            .map_err(Error::Pd)
    }

    async fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: TypeCStateMachineState,
    ) -> Result<(), Error<Self::BusError>> {
        let mut lock = self.state.lock().await;
        debug!("Set Type-C State Machine state for port {port:?}: {state:?}");
        lock.fn_calls.push_back(FnCall::SetTypeCStateMachineConfig(port, state));
        lock.next_result_set_type_c_state_machine_config
            .pop_front()
            .expect("next_result_set_type_c_state_machine_config not set")
            .map_err(Error::Pd)
    }

    async fn execute_ucsi_command(
        &mut self,
        command: lpm::LocalCommand,
    ) -> Result<Option<lpm::ResponseData>, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        debug!("Execute UCSI command for port {:?}: {command:?}", command.port());
        state.fn_calls.push_back(FnCall::ExecuteUcsiCommand(command));
        state
            .next_result_execute_ucsi_command
            .pop_front()
            .map(|r| r.map_err(Error::Pd))
            .expect("next_result_execute_ucsi_command not set")
    }

    async fn execute_electrical_disconnect(
        &mut self,
        port: LocalPortId,
        reconnect_time_s: Option<NonZeroU8>,
    ) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state
            .fn_calls
            .push_back(FnCall::ExecuteElectricalDisconnect(port, reconnect_time_s));
        debug!("Execute electrical disconnect for port {port:?} with reconnect time {reconnect_time_s:?}");
        state
            .next_result_execute_electrical_disconnect
            .pop_front()
            .expect("next_result_execute_electrical_disconnect not set")
            .map_err(Error::Pd)
    }

    async fn set_power_state(
        &mut self,
        port: LocalPortId,
        state: SystemPowerState,
    ) -> Result<(), Error<Self::BusError>> {
        let mut lock = self.state.lock().await;
        debug!("Set power state for port {port:?}: {state:?}");
        lock.fn_calls.push_back(FnCall::SetPowerState(port, state));
        lock.next_result_set_power_state
            .pop_front()
            .expect("next_result_set_power_state not set")
            .map_err(Error::Pd)
    }

    async fn get_discovered_svids(&mut self, port: LocalPortId) -> Result<DiscoveredSvids, Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetDiscoveredSvids(port));
        debug!("Get discovered SVIDs for port {port:?}");
        state
            .next_result_get_discovered_svids
            .pop_front()
            .expect("next_result_get_discovered_svids not set")
            .map_err(Error::Pd)
    }

    async fn hard_reset(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::HardReset(port));
        debug!("Hard reset for port {port:?}");
        state
            .next_result_hard_reset
            .pop_front()
            .expect("next_result_hard_reset not set")
            .map_err(Error::Pd)
    }

    async fn get_discover_identity_sop_response(
        &mut self,
        port: LocalPortId,
    ) -> Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop::ResponseVdos, Error<Self::BusError>>
    {
        let mut state = self.state.lock().await;
        state.fn_calls.push_back(FnCall::GetDiscoverIdentitySopResponse(port));
        debug!("Get Discover Identity SOP response for port {port:?}");
        state
            .next_result_get_discover_identity_sop_response
            .pop_front()
            .expect("next_result_get_discover_identity_sop_response not set")
            .map_err(Error::Pd)
    }

    async fn get_discover_identity_sop_prime_response(
        &mut self,
        port: LocalPortId,
    ) -> Result<
        embedded_usb_pd::vdm::structured::command::discover_identity::sop_prime::ResponseVdos,
        Error<Self::BusError>,
    > {
        let mut state = self.state.lock().await;
        state
            .fn_calls
            .push_back(FnCall::GetDiscoverIdentitySopPrimeResponse(port));
        debug!("Get Discover Identity SOP' response for port {port:?}");
        state
            .next_result_get_discover_identity_sop_prime_response
            .pop_front()
            .expect("next_result_get_discover_identity_sop_prime_response not set")
            .map_err(Error::Pd)
    }
}

pub struct Validator;

impl type_c_service::wrapper::FwOfferValidator for Validator {
    fn validate(
        &self,
        _current: embedded_cfu_protocol::protocol_definitions::FwVersion,
        _offer: &embedded_cfu_protocol::protocol_definitions::FwUpdateOffer,
    ) -> embedded_cfu_protocol::protocol_definitions::FwUpdateOfferResponse {
        // For this example, we always accept the new version
        FwUpdateOfferResponse::new_accept(HostToken::Driver)
    }
}

pub type Wrapper<'a> =
    type_c_service::wrapper::ControllerWrapper<'a, GlobalRawMutex, Mutex<GlobalRawMutex, Controller<'a>>, Validator>;
