//! This module contains the `Controller` trait. Any types that implement this trait can be used with the `ControllerWrapper` struct
//! which provides a bridge between various service messages and the actual controller functions.
use core::array::from_fn;
use core::future::{pending, Future};

use embassy_futures::select::{select4, select_array, Either4};
use embassy_sync::mutex::Mutex;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::cfu::component::CfuDevice;
use embedded_services::ipc::deferred;
use embedded_services::power::policy::device::StateKind;
use embedded_services::power::policy::{self, action};
use embedded_services::transformers::object::{Object, RefGuard, RefMutGuard};
use embedded_services::type_c::controller::{self, Controller, PortStatus};
use embedded_services::type_c::event::{PortEvent, PortNotificationSingle, PortPending, PortStatusChanged};
use embedded_services::GlobalRawMutex;
use embedded_services::SyncCell;
use embedded_services::{debug, error, info, trace, warn};
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId};

use crate::{PortEventStreamer, PortEventVariant};

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
    /// State used to keep track of where we are as we turn the event bitfields into a stream of events
    port_event_streaming_state: Option<PortEventStreamer>,
}

impl Default for InternalState {
    fn default() -> Self {
        Self {
            fw_update_state: cfu::FwUpdateState::Idle,
            port_event_streaming_state: None,
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

/// Wrapper events
pub enum Event<'a> {
    /// Port status changed
    PortStatusChanged(LocalPortId, PortStatusChanged),
    /// Port notification received
    PortNotification(LocalPortId, PortNotificationSingle),
    /// Power policy command received
    PowerPolicyCommand(
        LocalPortId,
        deferred::Request<'a, GlobalRawMutex, policy::device::CommandData, policy::device::InternalResponseData>,
    ),
    /// Command from TCPM
    ControllerCommand(deferred::Request<'a, GlobalRawMutex, controller::Command, controller::Response<'static>>),
    CfuEvent(cfu::Event),
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
    active_events: [SyncCell<PortEvent>; N],
    /// Trait object for validating firmware versions
    fw_version_validator: V,
    /// FW update ticker used to check for timeouts and recovery attempts
    fw_update_ticker: Mutex<GlobalRawMutex, embassy_time::Ticker>,
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
            active_events: [const { SyncCell::new(PortEvent::none()) }; N],
            fw_version_validator,
            fw_update_ticker: Mutex::new(embassy_time::Ticker::every(embassy_time::Duration::from_millis(
                DEFAULT_FW_UPDATE_TICK_INTERVAL_MS,
            ))),
        }
    }

    /// Get the power policy devices for this controller.
    pub fn power_policy_devices(&self) -> &[policy::device::Device] {
        &self.power
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

    /// Process port status changed events
    async fn process_port_status_changed(
        &self,
        controller: &mut C,
        local_port_id: LocalPortId,
        status_event: PortStatusChanged,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        let port_index = local_port_id.0 as usize;
        let mut event = self.active_events[port_index].get();
        let global_port_id = self
            .pd_controller
            .lookup_global_port(local_port_id)
            .map_err(Error::Pd)?;

        if status_event == PortStatusChanged::none() {
            event.status = PortStatusChanged::none();
            self.active_events[port_index].set(event);
            return Ok(());
        }

        let status = controller.get_port_status(local_port_id, true).await?;
        trace!("Port{} status: {:#?}", global_port_id.0, status);

        let power = self.get_power_device(local_port_id)?;
        trace!("Port{} status events: {:#?}", global_port_id.0, status_event);
        if status_event.plug_inserted_or_removed() {
            self.process_plug_event(controller, power, local_port_id, &status)
                .await?;
        }

        // Only notify power policy of a contract after Sink Ready event (always after explicit or implicit contract)
        if status_event.sink_ready() {
            self.process_new_consumer_contract(controller, power, local_port_id, &status)
                .await?;
        }

        if status_event.new_power_contract_as_provider() {
            self.process_new_provider_contract(global_port_id, power, &status)
                .await?;
        }

        self.active_events[port_index].set(event.union(status_event.into()));

        let mut pending = PortPending::none();
        pending.pend_port(port_index);
        self.pd_controller.notify_ports(pending).await;

        Ok(())
    }

    /// Wait for a pending port event
    async fn wait_port_pending(
        &self,
        controller: &mut C,
    ) -> Result<PortEventStreamer, Error<<C as Controller>::BusError>> {
        if self.state.lock().await.fw_update_state.in_progress() {
            // Don't process events while firmware update is in progress
            debug!("Firmware update in progress, ignoring port events");
            return pending().await;
        }

        if let Some(ref mut streamer) = self.state.lock().await.port_event_streaming_state {
            // If we're converting the bitfields into an event stream yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            Ok(*streamer)
        } else {
            // We aren't in the process of converting the bitfields into an event stream
            // Wait for the next event
            controller.wait_port_event().await?;
            let pending: PortPending = FromIterator::from_iter(0..N);
            Ok(PortEventStreamer::new(pending.into_iter()))
        }
    }

    pub async fn wait_next(&self) -> Result<Event<'_>, Error<<C as Controller>::BusError>> {
        loop {
            let event = {
                let mut controller = self.controller.lock().await;
                select4(
                    self.wait_port_pending(&mut controller),
                    self.wait_power_command(),
                    self.pd_controller.receive(),
                    self.wait_cfu_command(),
                )
                .await
            };
            match event {
                Either4::First(stream) => {
                    let mut stream = stream?;
                    if let Some((port_id, event)) = stream
                        .next(async |port_id| {
                            let mut controller = self.controller.lock().await;
                            let ret = controller.clear_port_events(LocalPortId(port_id as u8)).await;
                            ret
                        })
                        .await?
                    {
                        let port_id = LocalPortId(port_id as u8);
                        self.state.lock().await.port_event_streaming_state = Some(stream);
                        match event {
                            PortEventVariant::StatusChanged(status_event) => {
                                // Return a port status changed event
                                return Ok(Event::PortStatusChanged(port_id, status_event));
                            }
                            PortEventVariant::Notification(notification) => {
                                // Return a port notification event
                                return Ok(Event::PortNotification(port_id, notification));
                            }
                        }
                    } else {
                        self.state.lock().await.port_event_streaming_state = None;
                    }
                }
                Either4::Second((port, request)) => return Ok(Event::PowerPolicyCommand(port, request)),
                Either4::Third(request) => return Ok(Event::ControllerCommand(request)),
                Either4::Fourth(event) => return Ok(Event::CfuEvent(event)),
            }
        }
    }

    /// Top-level processing function
    /// Only call this fn from one place in a loop. Otherwise a deadlock could occur.
    pub async fn process_event(&self, event: Event<'_>) -> Result<(), Error<<C as Controller>::BusError>> {
        let mut controller = self.controller.lock().await;
        let mut state = self.state.lock().await;
        match event {
            Event::PortStatusChanged(port_id, status_event) => {
                self.process_port_status_changed(&mut controller, port_id, status_event)
                    .await
            }
            Event::PowerPolicyCommand(port, request) => {
                let response = self
                    .process_power_command(&mut controller, &mut state, port, &request.command)
                    .await;
                request.respond(response);
                Ok(())
            }
            Event::ControllerCommand(request) => {
                let response = self
                    .process_pd_command(&mut controller, &mut state, &request.command)
                    .await;
                request.respond(response);
                Ok(())
            }
            Event::CfuEvent(event) => match event {
                cfu::Event::Request(request) => {
                    let response = self.process_cfu_command(&mut controller, &mut state, &request).await;
                    self.send_cfu_response(response).await;
                    Ok(())
                }
                cfu::Event::RecoveryTick => {
                    // FW Update tick, process timeouts and recovery attempts
                    self.process_cfu_tick(&mut controller, &mut state).await;
                    Ok(())
                }
            },
            Event::PortNotification(_, _) => {
                // Nop for us
                Ok(())
            }
        }
    }

    /// Combined processing function
    pub async fn process_next_event(&self) -> Result<(), Error<<C as Controller>::BusError>> {
        let event = self.wait_next().await?;
        self.process_event(event).await
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
