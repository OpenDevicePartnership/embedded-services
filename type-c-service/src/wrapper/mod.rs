//! This module contains the `Controller` trait. Any types that implement this trait can be used with the `ControllerWrapper` struct
//! which provides a bridge between various service messages and the actual controller functions.
use core::array::from_fn;
use core::future::Future;

use embassy_futures::select::{select4, select_array, Either4};
use embassy_sync::mutex::Mutex;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::cfu::component::CfuDevice;
use embedded_services::power::policy::device::StateKind;
use embedded_services::power::policy::{self, action};
use embedded_services::transformers::object::{Object, RefGuard, RefMutGuard};
use embedded_services::type_c::controller::{self, Controller, PortStatus};
use embedded_services::type_c::event::{PortEventFlags, PortEventKind};
use embedded_services::GlobalRawMutex;
use embedded_services::SyncCell;
use embedded_services::{debug, error, info, trace, warn};
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId};

mod cfu;
mod pd;
mod power;

/// Base interval for checking for FW update timeouts and recovery attempts
pub const DEFAULT_FW_UPDATE_TICK_INTERVAL_MS: u64 = 5000;
/// Default number of ticks before we consider a firmware update to have timed out
/// 300 seconds at 5 seconds per tick
pub const DEFAULT_FW_UPDATE_TIMEOUT_TICKS: u8 = 60;

/// Internal wrapper state
pub struct InternalState {
    /// If we're currently doing a firmware update
    pub fw_update_state: cfu::FwUpdateState,
    /// FW update ticker used to check for timeouts and recovery attempts
    fw_update_ticker: embassy_time::Ticker,
}

impl Default for InternalState {
    fn default() -> Self {
        Self {
            fw_update_state: cfu::FwUpdateState::Idle,
            fw_update_ticker: embassy_time::Ticker::every(embassy_time::Duration::from_millis(
                DEFAULT_FW_UPDATE_TICK_INTERVAL_MS,
            )),
        }
    }
}

/// Trait for validating firmware versions before applying an update
// TODO: remove this once we have a better framework for OEM customization
// See https://github.com/OpenDevicePartnership/embedded-services/issues/326
pub trait FwOfferValidator {
    /// Determine if we are accepting the firmware update offer, returns a CFU offer response
    fn validate(&self, current: FwVersion, offer: &FwUpdateOffer) -> FwUpdateOfferResponse;
}

/// Takes an implementation of the `Controller` trait and wraps it with logic to handle
/// message passing and power-policy integration.
pub struct ControllerWrapper<'a, const N: usize, C: Controller, V: FwOfferValidator> {
    /// PD controller to interface with PD service
    pd_controller: controller::Device<'a>,
    /// Power policy devices to interface with power policy service
    power: [policy::device::Device; N],
    /// CFU device to interface with firmware update service
    cfu_device: CfuDevice,
    /// Internal state for the wrapper
    state: Mutex<GlobalRawMutex, InternalState>,
    controller: Mutex<GlobalRawMutex, C>,
    active_events: [SyncCell<PortEventKind>; N],
    /// Trait object for validating firmware versions
    fw_version_validator: V,
}

impl<'a, const N: usize, C: Controller, V: FwOfferValidator> ControllerWrapper<'a, N, C, V> {
    /// Create a new controller wrapper
    pub fn new(
        pd_controller: controller::Device<'a>,
        power: [policy::device::Device; N],
        cfu_device: CfuDevice,
        controller: C,
        fw_version_validator: V,
    ) -> Self {
        Self {
            pd_controller,
            power,
            cfu_device,
            state: Mutex::new(Default::default()),
            controller: Mutex::new(controller),
            active_events: [const { SyncCell::new(PortEventKind::none()) }; N],
            fw_version_validator,
        }
    }

    /// Handle a plug event
    async fn process_plug_event(
        &self,
        _controller: &mut C,
        power: &policy::device::Device,
        port: LocalPortId,
        status: &PortStatus,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        if port.0 > N as u8 {
            error!("Invalid port {}", port.0);
            return PdError::InvalidPort.into();
        }

        info!("Plug event");
        if status.is_connected() {
            info!("Plug inserted");

            // Recover if we're not in the correct state
            if power.state().await.kind() != StateKind::Detached {
                warn!("Power device not in detached state, recovering");
                if let Err(e) = power.detach().await {
                    error!("Error detaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            }

            if let Ok(state) = power.try_device_action::<action::Detached>().await {
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            } else {
                // This should never happen
                error!("Power device not in detached state");
                return PdError::InvalidMode.into();
            }
        } else {
            info!("Plug removed");
            if let Err(e) = power.detach().await {
                error!("Error detaching power device: {:?}", e);
                return PdError::Failed.into();
            };
        }

        Ok(())
    }

    /// Process port events
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    async fn process_event(&self, controller: &mut C, state: &mut InternalState) {
        let mut port_events = PortEventFlags::none();

        if state.fw_update_state.in_progress() {
            // Don't process events while firmware update is in progress
            debug!("Firmware update in progress, ignoring port events");
            return;
        }

        for port in 0..N {
            let local_port_id = LocalPortId(port as u8);
            let global_port_id = match self.pd_controller.lookup_global_port(local_port_id) {
                Ok(port) => port,
                Err(_) => {
                    error!("Invalid local port {}", local_port_id.0);
                    continue;
                }
            };

            let event = match controller.clear_port_events(local_port_id).await {
                Ok(event) => event,
                Err(_) => {
                    error!("Error clearing port events",);
                    continue;
                }
            };

            if event == PortEventKind::none() {
                self.active_events[port].set(PortEventKind::none());
                continue;
            }

            port_events.pend_port(global_port_id);

            let status = match controller.get_port_status(local_port_id, false).await {
                Ok(status) => status,
                Err(_) => {
                    error!("Port{}: Error getting port status", global_port_id.0);
                    continue;
                }
            };
            trace!("Port{} status: {:#?}", port, status);

            let power = match self.get_power_device(local_port_id) {
                Ok(power) => power,
                Err(_) => {
                    error!("Port{}: Error getting power device", global_port_id.0);
                    continue;
                }
            };

            trace!("Port{} Interrupt: {:#?}", global_port_id.0, event);
            if event.plug_inserted_or_removed()
                && self
                    .process_plug_event(controller, power, local_port_id, &status)
                    .await
                    .is_err()
            {
                error!("Port{}: Error processing plug event", global_port_id.0);
                continue;
            }

            // Only notify power policy of a contract after Sink Ready event (always after explicit or implicit contract)
            if event.sink_ready()
                && self
                    .process_new_consumer_contract(controller, power, local_port_id, &status)
                    .await
                    .is_err()
            {
                error!("Port{}: Error processing new consumer contract", global_port_id.0);
                continue;
            }

            if event.new_power_contract_as_provider()
                && self
                    .process_new_provider_contract(global_port_id, power, &status)
                    .await
                    .is_err()
            {
                error!("Port{}: Error processing new provider contract", global_port_id.0);
                continue;
            }

            self.active_events[port].set(event);
            trace!("Port{}: Active Event: {:#?}", port, event);
        }

        self.pd_controller.notify_ports(port_events).await;
    }

    /// Top-level processing function
    /// Only call this fn from one place in a loop. Otherwise a deadlock could occur.
    pub async fn process(&self) {
        let mut controller = self.controller.lock().await;
        let mut state = self.state.lock().await;
        match select4(
            controller.wait_port_event(),
            self.wait_power_command(),
            self.pd_controller.receive(),
            self.wait_cfu_command(&mut state),
        )
        .await
        {
            Either4::First(r) => match r {
                Ok(_) => self.process_event(&mut controller, &mut state).await,
                Err(_) => error!("Error waiting for port event"),
            },
            Either4::Second((request, port)) => {
                let response = self
                    .process_power_command(&mut controller, &mut state, port, &request.command)
                    .await;
                request.respond(response);
            }
            Either4::Third(request) => {
                let response = self
                    .process_pd_command(&mut controller, &mut state, &request.command)
                    .await;
                request.respond(response);
            }
            Either4::Fourth(request) => match request {
                Some(request) => {
                    let response = self.process_cfu_command(&mut controller, &mut state, &request).await;
                    self.send_cfu_response(response).await;
                }
                None => {
                    // FW Update tick, process timeouts and recovery attempts
                    self.process_cfu_tick(&mut controller, &mut state).await;
                }
            },
        }
    }

    /// Register all devices with their respective services
    pub async fn register(&'static self) -> Result<(), Error<<C as Controller>::BusError>> {
        for device in &self.power {
            policy::register_device(device).await.map_err(|_| {
                error!(
                    "Controller{}: Failed to register power device {}",
                    self.pd_controller.id().0,
                    device.id().0
                );
                Error::Pd(PdError::Failed)
            })?;
        }

        controller::register_controller(&self.pd_controller)
            .await
            .map_err(|_| {
                error!(
                    "Controller{}: Failed to register PD controller",
                    self.pd_controller.id().0
                );
                Error::Pd(PdError::Failed)
            })?;

        //TODO: Remove when we have a more general framework in place
        embedded_services::cfu::register_device(&self.cfu_device)
            .await
            .map_err(|_| {
                error!("Controller{}: Failed to register CFU device", self.pd_controller.id().0);
                Error::Pd(PdError::Failed)
            })?;
        Ok(())
    }
}

impl<'a, const N: usize, C: Controller, V: FwOfferValidator> Object<C> for ControllerWrapper<'a, N, C, V> {
    fn get_inner(&self) -> impl Future<Output = impl RefGuard<C>> {
        self.controller.lock()
    }

    fn get_inner_mut(&self) -> impl Future<Output = impl RefMutGuard<C>> {
        self.controller.lock()
    }
}
