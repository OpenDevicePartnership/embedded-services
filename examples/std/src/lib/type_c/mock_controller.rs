use embassy_sync::{mutex::Mutex, signal::Signal};
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOfferResponse, HostToken};
use embedded_services::{
    GlobalRawMutex,
    power::policy::PowerCapability,
    type_c::{
        controller::{
            AttnVdm, ControllerStatus, DpConfig, DpPinConfig, DpStatus, OtherVdm, PdStateMachineConfig, PortStatus,
            RetimerFwUpdateState, SendVdm, TbtConfig, TypeCStateMachineState, UsbControlConfig,
        },
        event::PortEvent,
    },
};
use embedded_usb_pd::{Error, ado::Ado};
use embedded_usb_pd::{LocalPortId, PdError};
use embedded_usb_pd::{PowerRole, type_c::Current};
use embedded_usb_pd::{type_c::ConnectionState, ucsi::lpm};
use log::{debug, info, trace};
use std::cell::Cell;

pub struct ControllerState {
    events: Signal<GlobalRawMutex, PortEvent>,
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

    /// Simulate a connection
    pub async fn connect(&self, role: PowerRole, capability: PowerCapability, debug: bool, unconstrained: bool) {
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
        *self.status.lock().await = status;

        let mut events = PortEvent::none();
        events.status.set_plug_inserted_or_removed(true);
        events.status.set_new_power_contract_as_consumer(true);
        events.status.set_sink_ready(true);
        self.events.signal(events);
    }

    /// Simulate a sink connecting
    pub async fn connect_sink(&self, capability: PowerCapability, unconstrained: bool) {
        self.connect(PowerRole::Sink, capability, false, unconstrained).await;
    }

    /// Simulate a disconnection
    pub async fn disconnect(&self) {
        *self.status.lock().await = PortStatus::default();

        let mut events = PortEvent::none();
        events.status.set_plug_inserted_or_removed(true);
        self.events.signal(events);
    }

    /// Simulate a debug accessory source connecting
    pub async fn connect_debug_accessory_source(&self, current: Current) {
        self.connect(PowerRole::Source, current.into(), true, false).await;
    }

    /// Simulate a PD alert
    pub async fn send_pd_alert(&self, ado: Ado) {
        *self.pd_alert.lock().await = Some(ado);

        let mut events = PortEvent::none();
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
    events: Cell<PortEvent>,
}

impl<'a> Controller<'a> {
    pub fn new(state: &'a ControllerState) -> Self {
        Self {
            state,
            events: Cell::new(PortEvent::none()),
        }
    }

    /// Function to demonstrate calling functions directly on the controller
    pub fn custom_function(&self) {
        info!("Custom function called on controller");
    }
}

impl embedded_services::type_c::controller::Controller for Controller<'_> {
    type BusError = ();

    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let events = self.state.events.wait().await;
        trace!("Port event: {events:#?}");
        self.events.set(events);
        Ok(())
    }

    async fn clear_port_events(&mut self, _port: LocalPortId) -> Result<PortEvent, Error<Self::BusError>> {
        let events = self.events.get();
        debug!("Clear port events: {events:#?}");
        self.events.set(PortEvent::none());
        Ok(events)
    }

    async fn get_port_status(&mut self, _port: LocalPortId) -> Result<PortStatus, Error<Self::BusError>> {
        debug!("Get port status: {:#?}", *self.state.status.lock().await);
        Ok(*self.state.status.lock().await)
    }

    async fn enable_sink_path(&mut self, _port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Enable sink path: {enable}");
        Ok(())
    }

    async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
        debug!("Get controller status");
        Ok(ControllerStatus {
            mode: "Test",
            valid_fw_bank: true,
            fw_version0: 0xbadf00d,
            fw_version1: 0xdeadbeef,
        })
    }

    async fn reset_controller(&mut self) -> Result<(), Error<Self::BusError>> {
        debug!("Reset controller");
        Ok(())
    }

    async fn get_rt_fw_update_status(
        &mut self,
        _port: LocalPortId,
    ) -> Result<RetimerFwUpdateState, Error<Self::BusError>> {
        debug!("Get retimer fw update status");
        Ok(RetimerFwUpdateState::Inactive)
    }

    async fn set_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("Set retimer fw update state");
        Ok(())
    }

    async fn clear_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("Clear retimer fw update state");
        Ok(())
    }

    async fn set_rt_compliance(&mut self, _port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("Set retimer compliance");
        Ok(())
    }

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, Error<Self::BusError>> {
        let pd_alert = self.state.pd_alert.lock().await;
        if let Some(ado) = *pd_alert {
            debug!("Port{}: Get PD alert: {ado:#?}", port.0);
            Ok(Some(ado))
        } else {
            debug!("Port{}: No PD alert", port.0);
            Ok(None)
        }
    }

    async fn set_unconstrained_power(
        &mut self,
        _port: LocalPortId,
        unconstrained: bool,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Set unconstrained power: {unconstrained}");
        Ok(())
    }

    async fn get_active_fw_version(&mut self) -> Result<u32, Error<Self::BusError>> {
        Ok(0)
    }

    async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        Ok(())
    }

    async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        Ok(())
    }

    async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        Ok(())
    }

    async fn write_fw_contents(&mut self, _offset: usize, _data: &[u8]) -> Result<(), Error<Self::BusError>> {
        Ok(())
    }

    async fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Set max sink voltage for port {}: {:?}", port.0, voltage_mv);
        Ok(())
    }

    async fn reconfigure_retimer(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("reconfigure_retimer(port: {port:?})");
        Ok(())
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("clear_dead_battery_flag(port: {port:?})");
        Ok(())
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<OtherVdm, Error<Self::BusError>> {
        debug!("Get other VDM for port {port:?}");
        Ok(OtherVdm::default())
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<AttnVdm, Error<Self::BusError>> {
        debug!("Get attention VDM for port {port:?}");
        Ok(AttnVdm::default())
    }

    async fn send_vdm(&mut self, port: LocalPortId, tx_vdm: SendVdm) -> Result<(), Error<Self::BusError>> {
        debug!("Send VDM for port {port:?}: {tx_vdm:?}");
        Ok(())
    }

    async fn set_usb_control(
        &mut self,
        port: LocalPortId,
        config: UsbControlConfig,
    ) -> Result<(), Error<Self::BusError>> {
        debug!(
            "set_usb_control(port: {port:?}, usb2: {}, usb3: {}, usb4: {})",
            config.usb2_enabled, config.usb3_enabled, config.usb4_enabled
        );
        Ok(())
    }

    async fn get_dp_status(&mut self, port: LocalPortId) -> Result<DpStatus, Error<Self::BusError>> {
        debug!("Get DisplayPort status for port {port:?}");
        Ok(DpStatus {
            alt_mode_entered: false,
            dfp_d_pin_cfg: DpPinConfig::default(),
        })
    }

    async fn set_dp_config(&mut self, port: LocalPortId, config: DpConfig) -> Result<(), Error<Self::BusError>> {
        debug!(
            "Set DisplayPort config for port {port:?}: enable={}, pin_cfg={:?}",
            config.enable, config.dfp_d_pin_cfg
        );
        Ok(())
    }

    async fn execute_drst(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        debug!("Execute PD Data Reset for port {port:?}");
        Ok(())
    }

    async fn set_tbt_config(&mut self, port: LocalPortId, config: TbtConfig) -> Result<(), Error<Self::BusError>> {
        debug!("Set Thunderbolt config for port {port:?}: {config:?}");
        Ok(())
    }

    async fn set_pd_state_machine_config(
        &mut self,
        port: LocalPortId,
        config: PdStateMachineConfig,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Set PD State Machine config for port {port:?}: {config:?}");
        Ok(())
    }

    async fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: TypeCStateMachineState,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Set Type-C State Machine state for port {port:?}: {state:?}");
        Ok(())
    }

    async fn execute_ucsi_command(
        &mut self,
        command: lpm::LocalCommand,
    ) -> Result<Option<lpm::ResponseData>, Error<Self::BusError>> {
        debug!("Execute UCSI command for port {:?}: {command:?}", command.port());
        match command.operation() {
            lpm::CommandData::GetConnectorStatus => Ok(Some(lpm::ResponseData::GetConnectorStatus(
                lpm::get_connector_status::ResponseData::default(),
            ))),
            _ => Err(PdError::UnrecognizedCommand.into()),
        }
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

pub type Wrapper<'a> = type_c_service::wrapper::ControllerWrapper<'a, GlobalRawMutex, Controller<'a>, Validator>;
