use crate::wrapper::backing::ReferencedStorage;
use crate::wrapper::{ControllerWrapper, FwOfferValidator};
use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::{PdCcPullUp, PpExtVbusSw, PpIntVbusSw};
use ::tps6699x::{PORT0, PORT1, TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use bitfield::bitfield;
use bitflags::bitflags;
use core::array::from_fn;
use core::future::Future;
use core::iter::zip;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_time::Delay;
use embedded_hal_async::i2c::I2c;
use embedded_services::power::policy::PowerCapability;
use embedded_services::type_c::ATTN_VDM_LEN;
use embedded_services::type_c::controller::{
    self, AttnVdm, Controller, ControllerStatus, DpPinConfig, OtherVdm, PortStatus, SendVdm, TbtConfig,
    TypeCStateMachineState, UsbControlConfig,
};
use embedded_services::type_c::event::PortEvent;
use embedded_services::{debug, error, info, trace, type_c, warn};
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::pdinfo::PowerPathStatus;
use embedded_usb_pd::pdo::{Common, Contract, Rdo, sink, source};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::ucsi::lpm;
use embedded_usb_pd::{DataRole, Error, LocalPortId, PdError, PlugOrientation, PowerRole};
use tps6699x::MAX_SUPPORTED_PORTS;
use tps6699x::asynchronous::embassy as tps6699x_drv;
use tps6699x::asynchronous::fw_update::UpdateTarget;
use tps6699x::asynchronous::fw_update::{
    BorrowedUpdater, BorrowedUpdaterInProgress, disable_all_interrupts, enable_port0_interrupts,
};
use tps6699x::command::{
    ReturnValue,
    vdms::{INITIATOR_WAIT_TIME_MS, MAX_NUM_DATA_OBJECTS, Version},
};
use tps6699x::fw_update::UpdateConfig as FwUpdateConfig;
use tps6699x::registers::port_config::TypeCStateMachine;

type Updater<'a, M, B> = BorrowedUpdaterInProgress<tps6699x_drv::Tps6699x<'a, M, B>>;

/// Firmware update state
struct FwUpdateState<'a, M: RawMutex, B: I2c> {
    /// Updater state
    updater: Updater<'a, M, B>,
    /// Interrupt guards to maintain during the update
    ///
    /// This value is never read, only used to keep the interrupt guard alive
    #[allow(dead_code)]
    guards: [Option<tps6699x_drv::InterruptGuard<'a, M, B>>; MAX_SUPPORTED_PORTS],
}

pub struct Tps6699x<'a, M: RawMutex, B: I2c> {
    port_events: heapless::Vec<PortEvent, MAX_SUPPORTED_PORTS>,
    tps6699x: tps6699x_drv::Tps6699x<'a, M, B>,
    update_state: Option<FwUpdateState<'a, M, B>>,
    /// Firmware update configuration
    fw_update_config: FwUpdateConfig,
}

impl<'a, M: RawMutex, B: I2c> Tps6699x<'a, M, B> {
    /// Create a new TPS6699x instance
    ///
    /// Returns `None` if the number of ports is invalid.
    pub fn try_new(
        tps6699x: tps6699x_drv::Tps6699x<'a, M, B>,
        num_ports: usize,
        fw_update_config: FwUpdateConfig,
    ) -> Option<Self> {
        if num_ports == 0 || num_ports > MAX_SUPPORTED_PORTS {
            None
        } else {
            Some(Self {
                // num_ports validated by branch
                port_events: heapless::Vec::from_iter((0..num_ports).map(|_| PortEvent::none())),
                tps6699x,
                update_state: None,
                fw_update_config,
            })
        }
    }
}

bitfield! {
    /// DFP VDO structure
    #[derive(Clone, Copy)]
    struct DfpVdo(u32);
    impl Debug;

    /// Port number (5 bits)
    pub u8, port_number, set_port_number: 4, 0;
    /// Host USB capability (3 bits)
    pub u8, host_capability, set_host_capability: 26, 24;
    /// DFP VDO version (3 bits)
    pub u8, version, set_version: 31, 29;
}

bitflags! {
    /// DisplayPort Pin Configuration bitmap
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PdDpPinConfig: u8 {
        /// No pin assignment
        const NONE = 0x00;
        /// 4L DP connection using USBC-USBC cable (Pin Assignment C)
        const C = 0x04;
        /// 2L USB + 2L DP connection using USBC-USBC cable (Pin Assignment D)
        const D = 0x08;
        /// 4L DP connection using USBC-DP cable (Pin Assignment E)
        const E = 0x10;
    }
}

impl From<u8> for PdDpPinConfig {
    fn from(value: u8) -> Self {
        PdDpPinConfig::from_bits_truncate(value)
    }
}

impl From<PdDpPinConfig> for DpPinConfig {
    fn from(value: PdDpPinConfig) -> Self {
        Self {
            pin_c: value.contains(PdDpPinConfig::C),
            pin_d: value.contains(PdDpPinConfig::D),
            pin_e: value.contains(PdDpPinConfig::E),
        }
    }
}

impl From<DpPinConfig> for PdDpPinConfig {
    fn from(value: DpPinConfig) -> Self {
        let mut config = PdDpPinConfig::NONE;
        if value.pin_c {
            config |= PdDpPinConfig::C;
        }
        if value.pin_d {
            config |= PdDpPinConfig::D;
        }
        if value.pin_e {
            config |= PdDpPinConfig::E;
        }
        config
    }
}

bitfield! {
    /// DisplayPort Alt Mode Configure structure
    /// Corresponds to ExtPDAltDpConfig_t in C
    #[derive(Clone, Copy, Debug)]
    pub struct PdDpAltConfig(u32);

    /// Select configuration (2 bits)
    pub u8, select_config, set_select_config: 1, 0;
    /// Signaling (4 bits)
    pub u8, signaling, set_signaling: 5, 2;
    /// Pin configuration (8 bits)
    pub u8, config_pin, set_config_pin: 15, 8;
    /// Reserved field 1 (16 bits)
    pub u16, reserved1, set_reserved1: 31, 16;
}

impl<M: RawMutex, B: I2c> Controller for Tps6699x<'_, M, B> {
    type BusError = B::Error;

    /// Controller reset
    async fn reset_controller(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut delay = Delay;
        self.tps6699x.reset(&mut delay).await?;

        Ok(())
    }

    /// Wait for an event on any port
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let interrupts = self
            .tps6699x
            .wait_interrupt_any(false, from_fn(|_| IntEventBus1::all()))
            .await;

        for (interrupt, event) in zip(interrupts.iter(), self.port_events.iter_mut()) {
            if *interrupt == IntEventBus1::new_zero() {
                continue;
            }

            {
                if interrupt.plug_event() {
                    debug!("Event: Plug event");
                    event.status.set_plug_inserted_or_removed(true);
                }
                if interrupt.source_caps_received() {
                    debug!("Event: Source Caps received");
                    event.status.set_source_caps_received(true);
                }

                if interrupt.sink_ready() {
                    debug!("Event: Sink ready");
                    event.status.set_sink_ready(true);
                }

                if interrupt.new_consumer_contract() {
                    debug!("Event: New contract as consumer, PD controller act as Sink");
                    // Port is consumer and power negotiation is complete
                    event.status.set_new_power_contract_as_consumer(true);
                }

                if interrupt.new_provider_contract() {
                    debug!("Event: New contract as provider, PD controller act as source");
                    // Port is provider and power negotiation is complete
                    event.status.set_new_power_contract_as_provider(true);
                }

                if interrupt.power_swap_completed() {
                    debug!("Event: power swap completed");
                    event.status.set_power_swap_completed(true);
                }

                if interrupt.data_swap_completed() {
                    debug!("Event: data swap completed");
                    event.status.set_data_swap_completed(true);
                }

                if interrupt.am_entered() {
                    debug!("Event: alt mode entered");
                    event.status.set_alt_mode_entered(true);
                }

                if interrupt.hard_reset() {
                    debug!("Event: hard reset");
                    event.status.set_pd_hard_reset(true);
                }

                if interrupt.crossbar_error() {
                    debug!("Event: crossbar error");
                    event.notification.set_usb_mux_error_recovery(true);
                }

                if interrupt.usvid_mode_entered() {
                    debug!("Event: user svid mode entered");
                    event.notification.set_custom_mode_entered(true);
                }

                if interrupt.usvid_mode_exited() {
                    debug!("Event: usvid mode exited");
                    event.notification.set_custom_mode_exited(true);
                }

                if interrupt.usvid_attention_vdm_received() {
                    debug!("Event: user svid attention vdm received");
                    event.notification.set_custom_mode_attention_received(true);
                }

                if interrupt.usvid_other_vdm_received() {
                    debug!("Event: user svid other vdm received");
                    event.notification.set_custom_mode_other_vdm_received(true);
                }

                if interrupt.discover_mode_completed() {
                    debug!("Event: discover mode completed");
                    event.notification.set_discover_mode_completed(true);
                }

                if interrupt.dp_sid_status_updated() {
                    debug!("Event: dp sid status updated");
                    event.notification.set_dp_status_update(true);
                }

                if interrupt.alert_message_received() {
                    debug!("Event: alert message received");
                    event.notification.set_alert(true);
                }
            }
        }
        Ok(())
    }

    /// Returns and clears current events for the given port
    ///
    /// Drop safety: All state changes happen after await point
    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEvent, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(core::mem::replace(
            &mut self.port_events[port.0 as usize],
            PortEvent::none(),
        ))
    }

    /// Returns the current status of the port
    async fn get_port_status(&mut self, port: LocalPortId) -> Result<PortStatus, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }

        let status = self.tps6699x.get_port_status(port).await?;
        trace!("Port{} status: {:#?}", port.0, status);

        let pd_status = self.tps6699x.get_pd_status(port).await?;
        trace!("Port{} PD status: {:#?}", port.0, pd_status);

        let port_control = self.tps6699x.get_port_control(port).await?;
        trace!("Port{} control: {:#?}", port.0, port_control);

        let mut port_status = PortStatus::default();

        let plug_present = status.plug_present();
        port_status.connection_state = status.connection_state().try_into().ok();

        debug!("Port{} Plug present: {}", port.0, plug_present);
        debug!("Port{} Valid connection: {}", port.0, port_status.is_connected());

        if port_status.is_connected() {
            // Determine current contract if any
            let pdo_raw = self.tps6699x.get_active_pdo_contract(port).await?.active_pdo();
            trace!("Raw PDO: {:#X}", pdo_raw);
            let rdo_raw = self.tps6699x.get_active_rdo_contract(port).await?.active_rdo();
            trace!("Raw RDO: {:#X}", rdo_raw);

            if pdo_raw != 0 && rdo_raw != 0 {
                // Got a valid explicit contract
                if pd_status.is_source() {
                    let pdo = source::Pdo::try_from(pdo_raw).map_err(|_| Error::from(PdError::InvalidParams))?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_source_contract = Contract::from_source(pdo, rdo).try_into().ok();
                    port_status.dual_power = pdo.dual_role_power();
                } else {
                    // active_rdo_contract doesn't contain the full picture
                    let mut source_pdos: [source::Pdo; 1] = [source::Pdo::default()];
                    // Read 5V fixed supply source PDO, guaranteed to be present as the first SPR PDO
                    let (num_sprs, _) = self
                        .tps6699x
                        .lock_inner()
                        .await
                        .get_rx_src_caps(port, &mut source_pdos[..], &mut [])
                        .await?;

                    if num_sprs == 0 {
                        // USB PD spec requires at least one source PDO be present, something is really wrong
                        error!("Port{} no source PDOs found", port.0);
                        return Err(PdError::InvalidParams.into());
                    }

                    let pdo = sink::Pdo::try_from(pdo_raw).map_err(|_| Error::from(PdError::InvalidParams))?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_sink_contract = Contract::from_sink(pdo, rdo).try_into().ok();
                    port_status.dual_power = source_pdos[0].dual_role_power();
                    port_status.unconstrained_power = source_pdos[0].unconstrained_power();
                }
            } else if pd_status.is_source() {
                // Implicit source contract
                let current = TypecCurrent::try_from(port_control.typec_current()).map_err(Error::Pd)?;
                debug!("Port{} type-C source current: {:#?}", port.0, current);
                let new_contract = Some(PowerCapability::from(current));
                port_status.available_source_contract = new_contract;
            } else {
                // Implicit sink contract
                let pull = pd_status.cc_pull_up();
                let new_contract = if pull == PdCcPullUp::NoPull {
                    // No pull up means no contract
                    debug!("Port{} no pull up", port.0);
                    None
                } else {
                    let current = TypecCurrent::try_from(pd_status.cc_pull_up()).map_err(Error::Pd)?;
                    debug!("Port{} type-C sink current: {:#?}", port.0, current);
                    Some(PowerCapability::from(current))
                };
                port_status.available_sink_contract = new_contract;
            }

            port_status.plug_orientation = if status.plug_orientation() {
                PlugOrientation::CC2
            } else {
                PlugOrientation::CC1
            };
            port_status.power_role = if status.port_role() {
                PowerRole::Source
            } else {
                PowerRole::Sink
            };
            port_status.data_role = if status.data_role() {
                DataRole::Dfp
            } else {
                DataRole::Ufp
            };

            // Update alt-mode status
            let alt_mode = self.tps6699x.get_alt_mode_status(port).await?;
            debug!("Port{} alt mode: {:#?}", port.0, alt_mode);
            port_status.alt_mode = alt_mode;

            // Update power path status
            let power_path = self.tps6699x.get_power_path_status(port).await?;
            trace!("Port{} power source: {:#?}", port.0, power_path);
            port_status.power_path = match port {
                PORT0 => PowerPathStatus::new(
                    power_path.pa_ext_vbus_sw() == PpExtVbusSw::EnabledInput,
                    power_path.pa_int_vbus_sw() == PpIntVbusSw::EnabledOutput,
                ),
                PORT1 => PowerPathStatus::new(
                    power_path.pb_ext_vbus_sw() == PpExtVbusSw::EnabledInput,
                    power_path.pb_int_vbus_sw() == PpIntVbusSw::EnabledOutput,
                ),
                _ => Err(PdError::InvalidPort)?,
            };
            debug!("Port{} power path: {:#?}", port.0, port_status.power_path);
        }

        Ok(port_status)
    }

    async fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::RetimerFwUpdateState, Error<Self::BusError>> {
        match self.tps6699x.get_rt_fw_update_status(port).await {
            Ok(true) => Ok(type_c::controller::RetimerFwUpdateState::Active),
            Ok(false) => Ok(type_c::controller::RetimerFwUpdateState::Inactive),
            Err(e) => Err(e),
        }
    }

    async fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        self.tps6699x.set_rt_fw_update_state(port).await
    }

    fn clear_rt_fw_update_state(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>> {
        self.tps6699x.clear_rt_fw_update_state(port)
    }

    async fn set_rt_compliance(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        self.tps6699x.set_rt_compliance(port).await
    }

    async fn reconfigure_retimer(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let input = {
            let mut input = tps6699x::command::muxr::Input(0);
            input.set_en_retry_on_target_addr_1(true);
            input
        };

        match self.tps6699x.execute_muxr(port, input).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing MuxR on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        match self.tps6699x.execute_dbfg(port).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing DBfg on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        match self.tps6699x.enable_sink_path(port, enable).await {
            // Temporary workaround for autofet rejection
            // Tracking bug: https://github.com/OpenDevicePartnership/embedded-services/issues/268
            Err(Error::Pd(PdError::Rejected)) | Err(Error::Pd(PdError::Timeout)) => {
                info!("Port{} autofet rejection, ignored", port.0);
                Ok(())
            }
            rest => rest,
        }
    }

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, Error<Self::BusError>> {
        self.tps6699x.get_rx_ado(port).await.map_err(Error::from)
    }

    async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
        let boot_flags = self.tps6699x.get_boot_flags().await?;
        let customer_use = CustomerUse(self.tps6699x.get_customer_use().await?);

        Ok(ControllerStatus {
            mode: self.tps6699x.get_mode().await?.into(),
            valid_fw_bank: (boot_flags.active_bank() == 0 && boot_flags.bank0_valid() != 0)
                || (boot_flags.active_bank() == 1 && boot_flags.bank1_valid() != 0),
            fw_version0: customer_use.ti_fw_version(),
            fw_version1: customer_use.custom_fw_version(),
        })
    }

    fn set_unconstrained_power(
        &mut self,
        port: LocalPortId,
        unconstrained: bool,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>> {
        self.tps6699x.set_unconstrained_power(port, unconstrained)
    }

    async fn get_active_fw_version(&mut self) -> Result<u32, Error<Self::BusError>> {
        let customer_use = CustomerUse(self.tps6699x.get_customer_use().await?);
        Ok(customer_use.custom_fw_version())
    }

    async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut delay = Delay;
        let mut updater: BorrowedUpdater<tps6699x_drv::Tps6699x<'_, M, B>> =
            BorrowedUpdater::with_config(self.fw_update_config.clone());

        // Abandon any previous in-progress update
        if let Some(update) = self.update_state.take() {
            warn!("Abandoning in-progress update");
            update
                .updater
                .abort_fw_update(&mut [&mut self.tps6699x], &mut delay)
                .await;
        }

        let mut guards = [const { None }; MAX_SUPPORTED_PORTS];
        // Disable all interrupts on both ports, use guards[1] to ensure that this set of guards is dropped last
        disable_all_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut self.tps6699x], &mut guards[1..]).await?;
        let in_progress = updater.start_fw_update(&mut [&mut self.tps6699x], &mut delay).await?;
        // Re-enable interrupts on port 0 only
        enable_port0_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut self.tps6699x], &mut guards[0..1])
            .await?;
        self.update_state = Some(FwUpdateState {
            updater: in_progress,
            guards,
        });
        Ok(())
    }

    /// Aborts the firmware update in progress
    ///
    /// This can reset the controller
    async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        // Check if we're still in firmware update mode
        if self.tps6699x.get_mode().await? == tps6699x::Mode::F211 {
            let mut delay = Delay;

            if let Some(update) = self.update_state.take() {
                // Attempt to abort the firmware update by consuming our update object
                update
                    .updater
                    .abort_fw_update(&mut [&mut self.tps6699x], &mut delay)
                    .await;
                Ok(())
            } else {
                // Bypass our update object since we've gotten into a state where we don't have one
                self.tps6699x.fw_update_mode_exit(&mut delay).await
            }
        } else {
            // Not in FW update mode, don't need to do anything
            Ok(())
        }
    }

    /// Finalize the firmware update
    ///
    /// This will reset the controller
    async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        if let Some(update) = self.update_state.take() {
            let mut delay = Delay;
            update
                .updater
                .complete_fw_update(&mut [&mut self.tps6699x], &mut delay)
                .await
        } else {
            Err(PdError::InvalidMode.into())
        }
    }

    async fn write_fw_contents(&mut self, _offset: usize, data: &[u8]) -> Result<(), Error<Self::BusError>> {
        if let Some(update) = &mut self.update_state {
            let mut delay = Delay;
            update
                .updater
                .write_bytes(&mut [&mut self.tps6699x], &mut delay, data)
                .await?;
            Ok(())
        } else {
            Err(PdError::InvalidMode.into())
        }
    }

    fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>> {
        self.tps6699x.set_autonegotiate_sink_max_voltage(port, voltage_mv)
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<OtherVdm, Error<Self::BusError>> {
        match self.tps6699x.get_rx_other_vdm(port).await {
            Ok(vdm) => Ok((*vdm.as_bytes()).into()),
            Err(e) => Err(e),
        }
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<AttnVdm, Error<Self::BusError>> {
        match self.tps6699x.get_rx_attn_vdm(port).await {
            Ok(vdm) => {
                let buf: [u8; ATTN_VDM_LEN] = vdm.into();
                let attn_vdm: AttnVdm = buf.into();
                Ok(attn_vdm)
            }
            Err(e) => Err(e),
        }
    }

    async fn send_vdm(&mut self, port: LocalPortId, tx_vdm: SendVdm) -> Result<(), Error<Self::BusError>> {
        let input = {
            let mut input = tps6699x::command::vdms::Input::default();
            input.set_num_vdo(tx_vdm.vdo_count);
            input.set_version(Version::Two);
            input.set_initiator(tx_vdm.initiator);
            if tx_vdm.initiator {
                input.set_initiator_wait_timer(INITIATOR_WAIT_TIME_MS);
            }

            for (index, vdo) in tx_vdm.vdo_data.iter().take(tx_vdm.vdo_count as usize).enumerate() {
                if index >= MAX_NUM_DATA_OBJECTS {
                    warn!("VDM data exceeds available VDO slots, truncating");
                    break; // Prevent out-of-bounds access
                }
                input.set_vdo(index, *vdo);
            }
            input
        };

        match self.tps6699x.send_vdms(port, input).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing VDMs on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    /// Set USB control configuration for the given port
    async fn set_usb_control(
        &mut self,
        port: LocalPortId,
        config: UsbControlConfig,
    ) -> Result<(), Error<Self::BusError>> {
        let mut tx_identity_value = 0;

        if config.usb2_enabled {
            tx_identity_value |= 1 << 0;
        }
        if config.usb3_enabled {
            tx_identity_value |= 1 << 1;
        }
        if config.usb4_enabled {
            tx_identity_value |= 1 << 2;
        }

        self.tps6699x
            .modify_tx_identity(port, |identity| {
                let mut dfp_vdo = DfpVdo(identity.dfp1_vdo());
                dfp_vdo.set_host_capability(tx_identity_value);
                identity.set_dfp1_vdo(dfp_vdo.0);
                identity.clone()
            })
            .await?;
        Ok(())
    }

    async fn get_dp_status(&mut self, port: LocalPortId) -> Result<controller::DpStatus, Error<Self::BusError>> {
        let dp_status = self.tps6699x.get_dp_status(port).await?;
        debug!("Port{} DP status: {:#?}", port.0, dp_status);

        let alt_mode_entered = dp_status.dp_mode_active() != 0;

        let dp_config = PdDpAltConfig(dp_status.dp_configure_message());
        let cfg_raw: PdDpPinConfig = dp_config.config_pin().into();
        let pin_config: DpPinConfig = cfg_raw.into();

        Ok(controller::DpStatus {
            alt_mode_entered,
            dfp_d_pin_cfg: pin_config,
        })
    }

    async fn set_dp_config(
        &mut self,
        port: LocalPortId,
        config: controller::DpConfig,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} setting DP config: {:#?}", port.0, config);

        let mut dp_config_reg = self.tps6699x.get_dp_config(port).await?;

        debug!("Current DP config: {:#?}", dp_config_reg);

        dp_config_reg.set_enable_dp_mode(config.enable);
        let cfg_raw: PdDpPinConfig = config.dfp_d_pin_cfg.into();
        dp_config_reg.set_dfpd_pin_assignment(cfg_raw.bits());

        self.tps6699x.set_dp_config(port, dp_config_reg).await?;
        Ok(())
    }

    async fn execute_drst(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        match self.tps6699x.execute_drst(port).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing DRST on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    async fn set_tbt_config(&mut self, port: LocalPortId, config: TbtConfig) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} setting TBT config: {:#?}", port.0, config);

        let mut config_reg = self.tps6699x.lock_inner().await.get_tbt_config(port).await?;

        config_reg.set_tbt_vid_en(config.tbt_enabled);
        config_reg.set_tbt_mode_en(config.tbt_enabled);

        self.tps6699x.lock_inner().await.set_tbt_config(port, config_reg).await
    }

    async fn set_pd_state_machine_config(
        &mut self,
        port: LocalPortId,
        config: controller::PdStateMachineConfig,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} setting PD state machine config: {:#?}", port.0, config);

        let mut config_reg = self.tps6699x.lock_inner().await.get_port_config(port).await?;

        config_reg.set_disable_pd(!config.enabled);

        self.tps6699x.lock_inner().await.set_port_config(port, config_reg).await
    }

    async fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: controller::TypeCStateMachineState,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} setting Type-C state machine state: {:#?}", port.0, state);

        let mut config_reg = self.tps6699x.lock_inner().await.get_port_config(port).await?;
        let typec_state = match state {
            TypeCStateMachineState::Sink => TypeCStateMachine::Sink,
            TypeCStateMachineState::Source => TypeCStateMachine::Source,
            TypeCStateMachineState::Drp => TypeCStateMachine::Drp,
            TypeCStateMachineState::Disabled => TypeCStateMachine::Disabled,
        };

        config_reg.set_typec_state_machine(typec_state);
        self.tps6699x.lock_inner().await.set_port_config(port, config_reg).await
    }

    async fn execute_ucsi_command(
        &mut self,
        command: lpm::LocalCommand,
    ) -> Result<Option<lpm::ResponseData>, Error<Self::BusError>> {
        self.tps6699x.execute_ucsi_command(&command).await
    }
}

impl<'a, M: RawMutex, BUS: I2c> AsRef<tps6699x_drv::Tps6699x<'a, M, BUS>> for Tps6699x<'a, M, BUS> {
    fn as_ref(&self) -> &tps6699x_drv::Tps6699x<'a, M, BUS> {
        &self.tps6699x
    }
}

impl<'a, M: RawMutex, BUS: I2c> AsMut<tps6699x_drv::Tps6699x<'a, M, BUS>> for Tps6699x<'a, M, BUS> {
    fn as_mut(&mut self) -> &mut tps6699x_drv::Tps6699x<'a, M, BUS> {
        &mut self.tps6699x
    }
}

/// TPS6699x controller wrapper
pub type Tps6699xWrapper<'a, M, BUS, V> = ControllerWrapper<'a, M, Tps6699x<'a, M, BUS>, V>;

/// Create a TPS66994 controller wrapper, returns `None` if the number of ports is invalid
pub fn tps66994<'a, M: RawMutex, BUS: I2c, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, BUS>,
    storage: &'a ReferencedStorage<'a, TPS66994_NUM_PORTS, M>,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Option<Tps6699xWrapper<'a, M, BUS, V>> {
    const _: () = assert!(
        TPS66994_NUM_PORTS > 0 && TPS66994_NUM_PORTS <= MAX_SUPPORTED_PORTS,
        "Number of ports exceeds maximum supported"
    );

    ControllerWrapper::try_new(
        // Statically checked above
        Tps6699x::try_new(controller, TPS66994_NUM_PORTS, fw_update_config).unwrap(),
        storage,
        fw_version_validator,
    )
}

/// Create a new TPS66993 controller wrapper, returns `None` if the number of ports is invalid
pub fn tps66993<'a, M: RawMutex, BUS: I2c, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, BUS>,
    backing: &'a ReferencedStorage<'a, TPS66993_NUM_PORTS, M>,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Option<Tps6699xWrapper<'a, M, BUS, V>> {
    const _: () = assert!(
        TPS66993_NUM_PORTS > 0 && TPS66993_NUM_PORTS <= MAX_SUPPORTED_PORTS,
        "Number of ports exceeds maximum supported"
    );
    ControllerWrapper::try_new(
        // Statically checked above
        Tps6699x::try_new(controller, TPS66993_NUM_PORTS, fw_update_config).unwrap(),
        backing,
        fw_version_validator,
    )
}

bitfield! {
    /// Custom customer use format
    //#[derive(Clone, Copy)]
    //#[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct CustomerUse(u64);
    impl Debug;
    /// Custom FW version
    pub u32, custom_fw_version, set_custom_fw_version: 31, 0;
    /// TI FW version
    pub u32, ti_fw_version, set_ti_fw_version: 63, 32;
}
