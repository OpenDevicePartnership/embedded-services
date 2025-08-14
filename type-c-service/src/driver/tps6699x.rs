use crate::wrapper::backing::Backing;
use crate::wrapper::{ControllerWrapper, FwOfferValidator};
use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::{PdCcPullUp, PpExtVbusSw, PpIntVbusSw};
use ::tps6699x::{PORT0, PORT1, TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use bitfield::bitfield;
use core::array::from_fn;
use core::future::Future;
use core::iter::zip;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Delay, Duration, Ticker, TimeoutError};
use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_hal_async::i2c::I2c;
use embedded_services::cfu::component::CfuDevice;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::transformers::object::{Object, RefGuard, RefMutGuard};
use embedded_services::type_c::controller::{self, Controller, ControllerStatus, PortStatus};
use embedded_services::type_c::event::{PortEvent, PortStatusChanged};
use embedded_services::type_c::ControllerId;
use embedded_services::{debug, error, info, trace, type_c, warn, GlobalRawMutex};
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::pdinfo::PowerPathStatus;
use embedded_usb_pd::pdo::{sink, source, Common, Rdo};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::{DataRole, Error, GlobalPortId, PdError, PlugOrientation, PortId as LocalPortId, PowerRole};
use tps6699x::asynchronous::embassy as tps6699x_drv;
use tps6699x::asynchronous::fw_update::UpdateTarget;
use tps6699x::asynchronous::fw_update::{
    disable_all_interrupts, enable_port0_interrupts, BorrowedUpdater, BorrowedUpdaterInProgress,
};
use tps6699x::command::ReturnValue;
use tps6699x::fw_update::UpdateConfig as FwUpdateConfig;
use tps6699x::registers::rx_other_vdm as Vdm;
use tps6699x::Mode;

type Updater<'a, M, B> = BorrowedUpdaterInProgress<tps6699x_drv::Tps6699x<'a, M, B>>;

/// The length of the `Rx Attention Vdm` register, in bytes.
pub const ATTN_VDM_LEN: usize = 9;

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
    port_events: [Mutex<GlobalRawMutex, PortEvent>; N],
    port_status: [Mutex<GlobalRawMutex, PortStatus>; N],
    sw_event: Signal<M, ()>,
    tps6699x: Mutex<GlobalRawMutex, tps6699x_drv::Tps6699x<'a, M, B>>,
    update_state: Mutex<GlobalRawMutex, Option<FwUpdateState<'a, M, B>>>,
    /// Firmware update configuration
    fw_update_config: FwUpdateConfig,
}

impl<'a, const N: usize, M: RawMutex, B: I2c> Tps6699x<'a, N, M, B> {
    pub fn new(tps6699x: tps6699x_drv::Tps6699x<'a, M, B>, fw_update_config: FwUpdateConfig) -> Self {
        Self {
            port_events: [const { Mutex::new(PortEvent::none()) }; N],
            port_status: [const { Mutex::new(PortStatus::new()) }; N],
            sw_event: Signal::new(),
            tps6699x: Mutex::new(tps6699x),
            update_state: Mutex::new(None),
            fw_update_config,
        }
    }

    /// Wait for the controller to enter [`Mode::App1`], indefinitely.
    async fn wait_app1_no_timeout(
        &self,
        tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>,
        period: Duration,
    ) -> Result<(), Error<B::Error>> {
        let mut ticker = Ticker::every(period);
        loop {
            if tps6699x.get_mode().await? == Mode::App1 {
                return Ok(());
            }

            ticker.next().await;
        }
    }

    /// Wait for the controller to enter [`Mode::App1`] within `timeout`.
    async fn wait_app1(&self, tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>) -> Result<(), Error<B::Error>> {
        let timeout = Duration::from_millis(800); // expected duration from power-on to APP1
        match with_timeout(timeout, self.wait_app1_no_timeout(tps6699x, Duration::from_millis(50))).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(TimeoutError) => {
                debug!(
                    "Port not in {:?} after {}ms timeout, waiting for app mode",
                    Mode::App1,
                    timeout.as_millis()
                );
                Err(Error::Pd(PdError::InvalidMode))
            }
        }
    }

    /// Reads and caches the current status of the port, returns any detected events
    async fn update_port_status(
        &self,
        tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>,
        port: LocalPortId,
    ) -> Result<PortStatusChanged, Error<B::Error>> {
        let events = PortStatusChanged::none();

        self.wait_app1(tps6699x).await?;

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
                    let pdo = source::Pdo::try_from(pdo_raw).map_err(|_| Error::from(PdError::InvalidParams))?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_source_contract = Some(PowerCapability::from(pdo));
                    port_status.dual_power = pdo.dual_role_power();
                } else {
                    // active_rdo_contract doesn't contain the full picture
                    let mut source_pdos: [source::Pdo; 1] = [source::Pdo::default()];
                    // Read 5V fixed supply source PDO, guaranteed to be present as the first SPR PDO
                    let (num_sprs, _) = tps6699x
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
                    port_status.available_sink_contract = Some(PowerCapability::from(pdo));
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
            let alt_mode = tps6699x.get_alt_mode_status(port).await?;
            debug!("Port{} alt mode: {:#?}", port.0, alt_mode);
            port_status.alt_mode = alt_mode;

            // Update power path status
            let power_path = tps6699x.get_power_path_status(port).await?;
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

        *self.port_status[port.0 as usize].lock().await = port_status;
        Ok(events)
    }

    /// Wait for an event on any port
    async fn wait_interrupt_event(
        &self,
        tps6699x: &mut tps6699x_drv::Tps6699x<'a, M, B>,
    ) -> Result<(), Error<B::Error>> {
        let interrupts = tps6699x
            .wait_interrupt_any(false, from_fn(|_| IntEventBus1::all()))
            .await;

        for (interrupt, mutex) in zip(interrupts.iter(), self.port_events.iter()) {
            if *interrupt == IntEventBus1::new_zero() {
                continue;
            }

            {
                let mut event = mutex.lock().await;
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

    /// Wait for a software event
    async fn wait_sw_event(&self) {
        self.sw_event.wait().await;
    }

    /// Signal an event on the given port
    async fn signal_event(&self, port: LocalPortId, event: PortEvent) {
        if port.0 >= self.port_events.len() as u8 {
            return;
        }

        {
            let mut guard = self.port_events[port.0 as usize].lock().await;
            let current = *guard;
            *guard = current.union(event);
        }
        self.sw_event.signal(());
    }
}

impl<const N: usize, M: RawMutex, B: I2c> Controller for Tps6699x<'_, N, M, B> {
    type BusError = B::Error;

    /// Controller specific initialization
    async fn sync_state(&mut self) -> Result<(), Error<Self::BusError>> {
        for i in 0..N {
            let port = LocalPortId(i as u8);
            let event: PortStatusChanged;
            {
                let mut tps6699x = self
                    .tps6699x
                    .try_lock()
                    .expect("Driver should not have been locked before this, thus infallible");
                event = self.update_port_status(&mut tps6699x, port).await?;
            }
            self.signal_event(port, event.into()).await;
        }

        Ok(())
    }

    /// Wait for an event on any port
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        let _ = select(self.wait_interrupt_event(&mut tps6699x), self.wait_sw_event()).await;

        for (i, mutex) in self.port_events.iter().enumerate() {
            let port = LocalPortId(i as u8);

            let mut guard = mutex.lock().await;

            let event = guard.union(self.update_port_status(&mut tps6699x, port).await?.into());

            // TODO: We get interrupts for certain status changes that don't currently map to a generic port event
            // Enable this when those get fleshed out
            // Ignore empty events
            /*if event == PortEventKind::NONE {
                continue;
            }*/

            trace!("Port{} event: {:#?}", i, event);
            *guard = event;
        }
        Ok(())
    }

    /// Returns and clears current events for the given port
    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEvent, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }
        let mut guard = self.port_events[port.0 as usize].lock().await;
        let port_events = *guard;
        *guard = PortEvent::none();
        Ok(port_events)
    }

    /// Returns the current status of the port
    async fn get_port_status(
        &mut self,
        port: LocalPortId,
        cached: bool,
    ) -> Result<type_c::controller::PortStatus, Error<Self::BusError>> {
        if port.0 >= self.port_status.len() as u8 {
            return PdError::InvalidPort.into();
        }

        // sync port status
        if !cached {
            debug!("update port status");

            let mut tps6699x = self
                .tps6699x
                .try_lock()
                .expect("Driver should not have been locked before this, thus infallible");

            let _ = self.update_port_status(&mut tps6699x, port).await;
        } else {
            debug!("using cached port status");
        }

        Ok(*self.port_status[port.0 as usize].lock().await)
    }

    async fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::RetimerFwUpdateState, Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        match tps6699x.get_rt_fw_update_status(port).await {
            Ok(true) => Ok(type_c::controller::RetimerFwUpdateState::Active),
            Ok(false) => Ok(type_c::controller::RetimerFwUpdateState::Inactive),
            Err(e) => Err(e),
        }
    }

    async fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.set_rt_fw_update_state(port).await
    }

    async fn clear_rt_fw_update_state(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.clear_rt_fw_update_state(port).await
    }

    async fn set_rt_compliance(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.set_rt_compliance(port).await
    }

    async fn reconfigure_retimer(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");

        let input = {
            let mut input = tps6699x::command::muxr::Input(0);
            input.set_en_retry_on_target_addr_1(true);
            input
        };

        match tps6699x.execute_muxr(port, input).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing MuxR on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");

        match tps6699x.execute_dbfg(port).await? {
            ReturnValue::Success => Ok(()),
            r => {
                debug!("Error executing DBfg on port {}: {:#?}", port.0, r);
                Err(Error::Pd(PdError::InvalidResponse))
            }
        }
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
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

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.get_rx_ado(port).await.map_err(Error::from)
    }

    async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
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

    async fn set_unconstrained_power(
        &mut self,
        port: LocalPortId,
        unconstrained: bool,
    ) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.set_unconstrained_power(port, unconstrained).await
    }

    async fn get_active_fw_version(&self) -> Result<u32, Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        let customer_use = CustomerUse(tps6699x.get_customer_use().await?);
        Ok(customer_use.custom_fw_version())
    }

    async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        let mut delay = Delay;
        let mut updater: BorrowedUpdater<tps6699x_drv::Tps6699x<'_, M, B>> =
            BorrowedUpdater::with_config(self.fw_update_config.clone());

        // Abandon any previous in-progress update
        if let Some(update) = self
            .update_state
            .try_lock()
            .expect("Update state should not have been locked before this, thus infallible")
            .take()
        {
            warn!("Abandoning in-progress update");
            update.updater.abort_fw_update(&mut [&mut tps6699x], &mut delay).await;
        }

        let mut guards = [const { None }; 2];
        // Disable all interrupts on both ports, use guards[1] to ensure that this set of guards is dropped last
        disable_all_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut tps6699x], &mut guards[1..]).await?;
        let in_progress = updater.start_fw_update(&mut [&mut tps6699x], &mut delay).await?;
        // Re-enable interrupts on port 0 only
        enable_port0_interrupts::<tps6699x_drv::Tps6699x<'_, M, B>>(&mut [&mut tps6699x], &mut guards[0..1]).await?;
        let mut state = self
            .update_state
            .try_lock()
            .expect("Update state should not have been locked before this, thus infallible");
        *state = Some(FwUpdateState {
            updater: in_progress,
            guards,
        });
        Ok(())
    }

    /// Aborts the firmware update in progress
    ///
    /// This can reset the controller
    async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        // Check if we're still in firmware update mode
        if tps6699x.get_mode().await? == tps6699x::Mode::F211 {
            let mut delay = Delay;

            if let Some(update) = self
                .update_state
                .try_lock()
                .expect("Update state should not have been locked before this, thus infallible")
                .take()
            {
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
    async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        if let Some(update) = self
            .update_state
            .try_lock()
            .expect("Update state should not have been locked before this, thus infallible")
            .take()
        {
            let mut delay = Delay;
            update
                .updater
                .complete_fw_update(&mut [&mut tps6699x], &mut delay)
                .await
        } else {
            Err(PdError::InvalidMode.into())
        }
    }

    async fn write_fw_contents(&mut self, _offset: usize, data: &[u8]) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        let mut update_state = self
            .update_state
            .try_lock()
            .expect("Update state should not have been locked before this, thus infallible");
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

    async fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        tps6699x.set_autonegotiate_sink_max_voltage(port, voltage_mv).await
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<[u8; Vdm::LEN], Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        match tps6699x.get_rx_other_vdm(port).await {
            Ok(vdm) => Ok(vdm.into()),
            Err(e) => Err(e),
        }
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<[u8; ATTN_VDM_LEN], Error<Self::BusError>> {
        let mut tps6699x = self
            .tps6699x
            .try_lock()
            .expect("Driver should not have been locked before this, thus infallible");
        match tps6699x.get_rx_attn_vdm(port).await {
            Ok(vdm) => Ok(vdm.into()),
            Err(e) => Err(e),
        }
    }
}

impl<'a, const N: usize, M: RawMutex, B: I2c> Object<tps6699x_drv::Tps6699x<'a, M, B>> for Tps6699x<'a, N, M, B> {
    fn get_inner(&self) -> impl Future<Output = impl RefGuard<tps6699x_drv::Tps6699x<'a, M, B>>> {
        self.tps6699x.lock()
    }

    fn get_inner_mut(&self) -> impl Future<Output = impl RefMutGuard<tps6699x_drv::Tps6699x<'a, M, B>>> {
        self.tps6699x.lock()
    }
}

/// TPS66994 controller wrapper
pub type Tps66994Wrapper<'a, M, BUS, BACK, V> =
    ControllerWrapper<'a, TPS66994_NUM_PORTS, Tps6699x<'a, TPS66994_NUM_PORTS, M, BUS>, BACK, V>;

/// TPS66993 controller wrapper
pub type Tps66993Wrapper<'a, M, BUS, BACK, V> =
    ControllerWrapper<'a, TPS66993_NUM_PORTS, Tps6699x<'a, TPS66993_NUM_PORTS, M, BUS>, BACK, V>;

/// Create a TPS66994 controller wrapper
// TODO: combine and trim down the number of parameters
#[allow(clippy::too_many_arguments)]
pub fn tps66994<'a, M: RawMutex, BUS: I2c, BACK: Backing<'a>, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, BUS>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66994_NUM_PORTS],
    cfu_id: ComponentId,
    backing: BACK,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Result<Tps66994Wrapper<'a, M, BUS, BACK, V>, PdError> {
    if port_ids.len() != TPS66994_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        CfuDevice::new(cfu_id),
        backing,
        Tps6699x::new(controller, fw_update_config),
        fw_version_validator,
    ))
}

/// Create a new TPS66993 controller wrapper
// TODO: combine and trim down the number of parameters
#[allow(clippy::too_many_arguments)]
pub fn tps66993<'a, M: RawMutex, BUS: I2c, BACK: Backing<'a>, V: FwOfferValidator>(
    controller: tps6699x_drv::Tps6699x<'a, M, BUS>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66993_NUM_PORTS],
    cfu_id: ComponentId,
    backing: BACK,
    fw_update_config: FwUpdateConfig,
    fw_version_validator: V,
) -> Result<Tps66993Wrapper<'a, M, BUS, BACK, V>, PdError> {
    if port_ids.len() != TPS66993_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        CfuDevice::new(cfu_id),
        backing,
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
