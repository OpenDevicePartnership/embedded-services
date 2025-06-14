use core::array::from_fn;
use core::cell::{Cell, RefCell};
use core::iter::zip;

use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::{PdCcPullUp, PpExtVbusSw, PpIntVbusSw};
use ::tps6699x::{PORT0, PORT1, TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use bitfield::bitfield;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Delay;
use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_hal_async::i2c::I2c;
use embedded_services::cfu::component::CfuDevice;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::type_c::controller::{self, Controller, ControllerStatus, PortStatus};
use embedded_services::type_c::event::PortEventKind;
use embedded_services::type_c::ControllerId;
use embedded_services::{debug, info, trace, type_c, warn};
use embedded_usb_pd::pdinfo::PowerPathStatus;
use embedded_usb_pd::pdo::{sink, source, Common, Rdo};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::{Error, GlobalPortId, PdError, PortId as LocalPortId, PowerRole};
use tps6699x::asynchronous::embassy as tps6699x_drv;
use tps6699x::asynchronous::fw_update::UpdateTarget;
use tps6699x::asynchronous::fw_update::{
    disable_all_interrupts, enable_port0_interrupts, BorrowedUpdater, BorrowedUpdaterInProgress,
};
use tps6699x::fw_update::UpdateConfig as FwUpdateConfig;

type Updater<'a, M, B> = BorrowedUpdaterInProgress<tps6699x_drv::Tps6699x<'a, M, B>>;

use crate::wrapper::{ControllerWrapper, FwOfferValidator};

/// Firmware update state
struct FwUpdateState<'a, M: RawMutex, B: I2c> {
    /// Updater state
    updater: Updater<'a, M, B>,
    /// Interrupt guards to maintain during the update
    ///
    /// This value is never read, only used to keep the interrupt guard alive
    #[allow(dead_code)]
    guards: [Option<tps6699x_drv::InterruptGuard<'a, M, B>>; 2],
}

pub struct Tps6699x<'a, const N: usize, M: RawMutex, B: I2c> {
    port_events: [Cell<PortEventKind>; N],
    port_status: [Cell<PortStatus>; N],
    sw_event: Signal<M, ()>,
    tps6699x: RefCell<tps6699x_drv::Tps6699x<'a, M, B>>,
    update_state: RefCell<Option<FwUpdateState<'a, M, B>>>,
    /// Firmware update configuration
    fw_update_config: FwUpdateConfig,
}

impl<'a, const N: usize, M: RawMutex, B: I2c> Tps6699x<'a, N, M, B> {
    pub fn new(tps6699x: tps6699x_drv::Tps6699x<'a, M, B>, fw_update_config: FwUpdateConfig) -> Self {
        Self {
            port_events: [const { Cell::new(PortEventKind::none()) }; N],
            port_status: [const { Cell::new(PortStatus::new()) }; N],
            sw_event: Signal::new(),
            tps6699x: RefCell::new(tps6699x),
            update_state: RefCell::new(None),
            fw_update_config,
        }
    }

    /// Reads and caches the current status of the port, returns any detected events
    async fn update_port_status(
        &self,
        tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>,
        port: LocalPortId,
    ) -> Result<PortEventKind, Error<B::Error>> {
        let mut events = PortEventKind::none();
        let previous_status = self.port_status[port.0 as usize].get();

        let status = tps6699x.get_port_status(port).await?;
        trace!("Port{} status: {:#?}", port.0, status);

        let pd_status = tps6699x.get_pd_status(port).await?;
        trace!("Port{} PD status: {:#?}", port.0, pd_status);

        let port_control = tps6699x.get_port_control(port).await?;
        trace!("Port{} control: {:#?}", port.0, port_control);

        let mut port_status = PortStatus::default();

        let plug_present = status.plug_present();
        port_status.connection_state = status.connection_state().try_into().ok();

        debug!("Port{} Plug present: {}", port.0, plug_present);
        debug!("Port{} Valid connection: {}", port.0, port_status.is_connected());

        if port_status.is_connected() {
            // Determine current contract if any
            let pdo_raw = tps6699x.get_active_pdo_contract(port).await?.active_pdo();
            trace!("Raw PDO: {:#X}", pdo_raw);
            let rdo_raw = tps6699x.get_active_rdo_contract(port).await?.active_rdo();
            trace!("Raw RDO: {:#X}", rdo_raw);

            if pdo_raw != 0 && rdo_raw != 0 {
                // Got a valid explicit contract
                if pd_status.is_source() {
                    let pdo = source::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_source_contract = Some(PowerCapability::from(pdo));
                    port_status.dual_power = pdo.is_dual_role();
                } else {
                    let pdo = sink::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_sink_contract = Some(PowerCapability::from(pdo));
                    port_status.dual_power = pdo.is_dual_role()
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

            // Update alt-mode status
            let alt_mode = tps6699x.get_alt_mode_status(port).await?;
            debug!("Port{} alt mode: {:#?}", port.0, alt_mode);
            port_status.alt_mode = alt_mode;

            // Update power path status
            let power_path = tps6699x.get_power_path_status(port).await?;
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

        if port_status.available_sink_contract.is_some()
            && port_status.available_sink_contract != previous_status.available_sink_contract
        {
            debug!("Port{}: new sink contract", port.0);
            events.set_new_power_contract_as_consumer(true);
        }

        if port_status.available_source_contract.is_some()
            && port_status.available_source_contract != previous_status.available_source_contract
        {
            debug!("Port{}: new source contract", port.0);
            events.set_new_power_contract_as_provider(true);
        }

        self.port_status[port.0 as usize].set(port_status);
        Ok(events)
    }

    /// Wait for an event on any port
    async fn wait_interrupt_event(
        &self,
        tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>,
    ) -> Result<(), Error<B::Error>> {
        let interrupts = tps6699x.wait_interrupt(false, |_, _| true).await;

        for (interrupt, cell) in zip(interrupts.iter(), self.port_events.iter()) {
            if *interrupt == IntEventBus1::new_zero() {
                continue;
            }

            let mut event = cell.get();
            if interrupt.plug_event() {
                debug!("Plug event");
                event.set_plug_inserted_or_removed(true);
            }

            if interrupt.new_consumer_contract() {
                debug!("New consumer contract");
                event.set_new_power_contract_as_consumer(true);
            }

            if interrupt.new_provider_contract() {
                debug!("New consumer contract");
                event.set_new_power_contract_as_provider(true);
            }

            cell.set(event);
        }
        Ok(())
    }

    /// Wait for a software event
    async fn wait_sw_event(&self) {
        self.sw_event.wait().await;
    }

    /// Signal an event on the given port
    fn signal_event(&self, port: LocalPortId, event: PortEventKind) {
        if port.0 >= self.port_events.len() as u8 {
            return;
        }

        let cell = &self.port_events[port.0 as usize];
        let current = cell.get();
        cell.set(current.union(event));
        self.sw_event.signal(());
    }
}

impl<const N: usize, M: RawMutex, B: I2c> Controller for Tps6699x<'_, N, M, B> {
    type BusError = B::Error;

    /// Controller specific initialization
    #[allow(clippy::await_holding_refcell_ref)]
    async fn sync_state(&mut self) -> Result<(), Error<Self::BusError>> {
        for i in 0..N {
            let port = LocalPortId(i as u8);
            let mut tps6699x = self.tps6699x.borrow_mut();
            let event = self.update_port_status(&mut tps6699x, port).await?;
            self.signal_event(port, event);
        }

        Ok(())
    }

    /// Wait for an event on any port
    #[allow(clippy::await_holding_refcell_ref)]
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let _ = select(self.wait_interrupt_event(&mut tps6699x), self.wait_sw_event()).await;

        for (i, cell) in self.port_events.iter().enumerate() {
            let port = LocalPortId(i as u8);

            let event = cell.get().union(self.update_port_status(&mut tps6699x, port).await?);

            // TODO: We get interrupts for certain status changes that don't currently map to a generic port event
            // Enable this when those get fleshed out
            // Ignore empty events
            /*if event == PortEventKind::NONE {
                continue;
            }*/

            trace!("Port{} event: {:#?}", i, event);
            cell.set(event);
        }
        Ok(())
    }

    /// Returns and clears current events for the given port
    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEventKind, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(self.port_events[port.0 as usize].replace(PortEventKind::none()))
    }

    /// Returns the current status of the port
    async fn get_port_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::PortStatus, Error<Self::BusError>> {
        if port.0 >= self.port_status.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(self.port_status[port.0 as usize].get())
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::RetimerFwUpdateState, Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        match tps6699x.get_rt_fw_update_status(port).await {
            Ok(true) => Ok(type_c::controller::RetimerFwUpdateState::Active),
            Ok(false) => Ok(type_c::controller::RetimerFwUpdateState::Inactive),
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.set_rt_fw_update_state(port).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn clear_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.clear_rt_fw_update_state(port).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_rt_compliance(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.set_rt_compliance(port).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        match tps6699x.enable_sink_path(port, enable).await {
            // Temporary workaround for autofet rejection
            // Tracking bug: https://github.com/OpenDevicePartnership/embedded-services/issues/268
            Err(Error::Pd(PdError::Rejected)) | Err(Error::Pd(PdError::Timeout)) => {
                info!("Port{} autofet rejection, ignored", port.0);
                Ok(())
            }
            rest => rest,
        }
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_sourcing(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable source: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.enable_source(port, enable).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_source_current(
        &mut self,
        port: LocalPortId,
        current: TypecCurrent,
        signal_event: bool,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} set source current: {:?}", port.0, current);

        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut port_control = tps6699x.get_port_control(port).await?;
        port_control.set_typec_current(current.into());

        tps6699x.set_port_control(port, port_control).await?;
        if signal_event {
            let mut event = PortEventKind::none();
            event.set_new_power_contract_as_consumer(true);
            self.signal_event(port, event);
        }
        Ok(())
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn request_pr_swap(
        &mut self,
        port: LocalPortId,
        role: embedded_usb_pd::PowerRole,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} request PR swap to {:?}", port.0, role);

        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut control = tps6699x.get_port_control(port).await?;
        match role {
            PowerRole::Sink => control.set_initiate_swap_to_sink(true),
            PowerRole::Source => control.set_initiate_swap_to_source(true),
        }

        tps6699x.set_port_control(port, control).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let boot_flags = tps6699x.get_boot_flags().await?;
        let customer_use = CustomerUse(tps6699x.get_customer_use().await?);

        Ok(ControllerStatus {
            mode: tps6699x.get_mode().await?.into(),
            valid_fw_bank: (boot_flags.active_bank() == 0 && boot_flags.bank0_valid() != 0)
                || (boot_flags.active_bank() == 1 && boot_flags.bank1_valid() != 0),
            fw_version0: customer_use.ti_fw_version(),
            fw_version1: customer_use.custom_fw_version(),
        })
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn get_active_fw_version(&self) -> Result<u32, Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let customer_use = CustomerUse(tps6699x.get_customer_use().await?);
        Ok(customer_use.custom_fw_version())
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut delay = Delay;
        let mut updater: BorrowedUpdater<tps6699x_drv::Tps6699x<'_, M, B>> =
            BorrowedUpdater::with_config(self.fw_update_config.clone());

        // Abandon any previous in-progress update
        if self.update_state.borrow().is_some() {
            warn!("Abandoning in-progress update");
            return Err(Error::Pd(PdError::InvalidMode));
        }

        let mut guards = [const { None }; 2];
        // Disable all interrupts on both ports, use guards[1] to ensure that this set of guards is dropped last
        disable_all_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut tps6699x], &mut guards[1..]).await?;
        let in_progress = updater.start_fw_update(&mut [&mut tps6699x], &mut delay).await?;
        // Re-enable interrupts on port 0 only
        enable_port0_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut tps6699x], &mut guards[0..1]).await?;
        self.update_state.replace(Some(FwUpdateState {
            updater: in_progress,
            guards,
        }));
        Ok(())
    }

    /// Aborts the firmware update in progress
    ///
    /// This can reset the controller
    #[allow(clippy::await_holding_refcell_ref)]
    async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        // Check if we're still in firmware update mode
        if tps6699x.get_mode().await? == tps6699x::Mode::F211 {
            let mut delay = Delay;
            if let Some(update) = self.update_state.replace(None) {
                // Attempt to abort the firmware update by consuming our update object
                update.updater.abort_fw_update(&mut [&mut tps6699x], &mut delay).await;
                Ok(())
            } else {
                // Bypass our update object since we've gotten into a state where we don't have one
                tps6699x.fw_update_mode_exit(&mut delay).await
            }
        } else {
            // Not in FW update mode, don't need to do anything
            Ok(())
        }
    }

    /// Finalize the firmware update
    ///
    /// This will reset the controller
    #[allow(clippy::await_holding_refcell_ref)]
    async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        if let Some(update) = self.update_state.replace(None) {
            let mut delay = Delay;
            update
                .updater
                .complete_fw_update(&mut [&mut tps6699x], &mut delay)
                .await
        } else {
            Err(PdError::InvalidMode.into())
        }
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn write_fw_contents(&mut self, _offset: usize, data: &[u8]) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut update_state = self.update_state.borrow_mut();
        if let Some(update) = update_state.as_mut() {
            let mut delay = Delay;
            update
                .updater
                .write_bytes(&mut [&mut tps6699x], &mut delay, data)
                .await?;
            Ok(())
        } else {
            Err(PdError::InvalidMode.into())
        }
    }
}

/// TPS66994 controller wrapper
pub type Tps66994Wrapper<'a, M, B, V> =
    ControllerWrapper<'a, TPS66994_NUM_PORTS, Tps6699x<'a, TPS66994_NUM_PORTS, M, B>, V>;

/// TPS66993 controller wrapper
pub type Tps66993Wrapper<'a, M, B, V> =
    ControllerWrapper<'a, TPS66993_NUM_PORTS, Tps6699x<'a, TPS66993_NUM_PORTS, M, B>, V>;

/// Create a TPS66994 controller wrapper
pub fn tps66994<'a, M: RawMutex, B: I2c, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66994_NUM_PORTS],
    cfu_id: ComponentId,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Result<Tps66994Wrapper<'a, M, B, V>, PdError> {
    if port_ids.len() != TPS66994_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        CfuDevice::new(cfu_id),
        Tps6699x::new(controller, fw_update_config),
        fw_version_validator,
    ))
}

/// Create a new TPS66993 controller wrapper
pub fn tps66993<'a, M: RawMutex, B: I2c, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66993_NUM_PORTS],
    cfu_id: ComponentId,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Result<Tps66993Wrapper<'a, M, B, V>, PdError> {
    if port_ids.len() != TPS66993_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        CfuDevice::new(cfu_id),
        Tps6699x::new(controller, fw_update_config),
        fw_version_validator,
    ))
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
