//! This module contains the [`ControllerWrapper`] struct. This struct serves as a bridge between various service messages
//! and the actual controller functions provided by [`embedded_services::type_c::controller::Controller`].
//! # Supported service messaging
//! This struct current currently supports messages from the following services:
//! * Type-C: [`embedded_services::type_c::controller::Command`]
//! * Power policy: [`embedded_services::power::policy::device::Command`]
//! * CFU: [`embedded_services::cfu::Request`]
//! # Event loop
//! This struct follows a standard wait/process/finalize event loop.
//!
//! [`ControllerWrapper::wait_next`] returns [`message::Event`] and does not perform any actions on the controller
//! aside from reading pending events.
//!
//! [`ControllerWrapper::process_event`] reads any additional data relevant to the event and returns [`message::Output`].
//! e.g. port status for a port status changed event, VDM data for a VDM event
//!
//! [`ControllerWrapper::process_event`] consumes [`message::Output`] and responds to any deferred requests, performs
//! any caching/buffering of data, and notifies the type-C service implementation of the event if needed.
use core::array::from_fn;
use core::future::{pending, Future};

use embassy_futures::select::{select, select5, select_array, Either, Either5};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::cfu::component::CfuDevice;
use embedded_services::power::policy::device::StateKind;
use embedded_services::power::policy::{self, action};
use embedded_services::transformers::object::{Object, RefGuard, RefMutGuard};
use embedded_services::type_c::controller::{self, Controller, PortStatus};
use embedded_services::type_c::event::{PortEvent, PortNotificationSingle, PortPending, PortStatusChanged};
use embedded_services::GlobalRawMutex;
use embedded_services::{debug, error, info, trace, warn};
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::{Error, LocalPortId, PdError};

use crate::wrapper::backing::Backing;
use crate::wrapper::message::*;
use crate::{PortEventStreamer, PortEventVariant};

pub mod backing;
mod cfu;
mod dp;
pub mod message;
mod pd;
mod power;
mod vdm;

/// Base interval for checking for FW update timeouts and recovery attempts
pub const DEFAULT_FW_UPDATE_TICK_INTERVAL_MS: u64 = 5000;
/// Default number of ticks before we consider a firmware update to have timed out
/// 300 seconds at 5 seconds per tick
pub const DEFAULT_FW_UPDATE_TIMEOUT_TICKS: u8 = 60;

/// Internal per-port state
#[derive(Copy, Clone, Default)]
pub struct PortState {
    /// Cached port status
    status: PortStatus,
    /// Software status event
    sw_status_event: PortStatusChanged,
    /// Sink ready deadline instant
    sink_ready_deadline: Option<Instant>,
    /// Pending events for the type-C service
    pending_events: PortEvent,
}

/// Internal wrapper state
#[derive(Copy, Clone)]
pub struct InternalState<const N: usize> {
    /// If we're currently doing a firmware update
    pub fw_update_state: cfu::FwUpdateState,
    /// State used to keep track of where we are as we turn the event bitfields into a stream of events
    port_event_streaming_state: Option<PortEventStreamer>,
    /// Per-port state
    port_states: [PortState; N],
}

impl<const N: usize> Default for InternalState<N> {
    fn default() -> Self {
        Self {
            fw_update_state: cfu::FwUpdateState::Idle,
            port_event_streaming_state: None,
            port_states: [Default::default(); N],
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

/// Common functionality implemented on top of [`embedded_services::type_c::controller::Controller`]
pub struct ControllerWrapper<'a, const N: usize, C: Controller, BACK: Backing<'a>, V: FwOfferValidator> {
    /// PD controller to interface with PD service
    pd_controller: controller::Device<'a>,
    /// Power policy devices to interface with power policy service
    power: [policy::device::Device; N],
    /// CFU device to interface with firmware update service
    cfu_device: CfuDevice,
    /// Internal state for the wrapper
    state: Mutex<GlobalRawMutex, InternalState<N>>,
    controller: Mutex<GlobalRawMutex, C>,
    /// Trait object for validating firmware versions
    fw_version_validator: V,
    /// FW update ticker used to check for timeouts and recovery attempts
    fw_update_ticker: Mutex<GlobalRawMutex, embassy_time::Ticker>,
    /// Channels and buffers used by the wrapper
    backing: Mutex<GlobalRawMutex, BACK>,
    /// SW port status event signal
    sw_status_event: Signal<GlobalRawMutex, ()>,
}

impl<'a, const N: usize, C: Controller, BACK: Backing<'a>, V: FwOfferValidator> ControllerWrapper<'a, N, C, BACK, V> {
    /// Create a new controller wrapper
    pub fn new(
        pd_controller: controller::Device<'a>,
        power: [policy::device::Device; N],
        cfu_device: CfuDevice,
        backing: BACK,
        controller: C,
        fw_version_validator: V,
    ) -> Self {
        Self {
            pd_controller,
            power,
            cfu_device,
            state: Mutex::new(Default::default()),
            controller: Mutex::new(controller),
            fw_version_validator,
            fw_update_ticker: Mutex::new(embassy_time::Ticker::every(embassy_time::Duration::from_millis(
                DEFAULT_FW_UPDATE_TICK_INTERVAL_MS,
            ))),
            backing: Mutex::new(backing),
            sw_status_event: Signal::new(),
        }
    }

    /// Get the power policy devices for this controller.
    pub fn power_policy_devices(&self) -> &[policy::device::Device] {
        &self.power
    }

    /// Get the cached port status
    pub async fn get_cached_port_status(&self, local_port: LocalPortId) -> PortStatus {
        self.state.lock().await.port_states[local_port.0 as usize].status
    }

    /// Synchronize the state between the controller and the internal state
    pub async fn sync_state(&self) -> Result<(), Error<<C as Controller>::BusError>> {
        let mut controller = self.controller.lock().await;
        let mut state = self.state.lock().await;
        self.sync_state_internal(&mut controller, &mut state).await
    }

    /// Synchronize the state between the controller and the internal state
    async fn sync_state_internal(
        &self,
        controller: &mut C,
        state: &mut InternalState<N>,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        // Sync the controller state with the PD controller
        for port in 0..N {
            let mut status_changed = state.port_states[port].sw_status_event;
            let local_port = LocalPortId(port as u8);
            let status = controller.get_port_status(local_port).await?;
            trace!("Port{} status: {:#?}", port, status);

            let previous_status = state.port_states[port].status;

            if previous_status.is_connected() != status.is_connected() {
                status_changed.set_plug_inserted_or_removed(true);
            }

            if previous_status.available_sink_contract != status.available_sink_contract {
                status_changed.set_new_power_contract_as_consumer(true);
            }

            if previous_status.available_source_contract != status.available_source_contract {
                status_changed.set_new_power_contract_as_provider(true);
            }

            state.port_states[port].sw_status_event = status_changed;
            if state.port_states[port].sw_status_event != PortStatusChanged::none() {
                // Have a status changed event, notify
                trace!("Port{} status changed: {:#?}", port, status);
                self.sw_status_event.signal(());
            }
        }
        Ok(())
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
    async fn process_port_status_changed<'b>(
        &self,
        controller: &mut C,
        state: &mut InternalState<N>,
        local_port_id: LocalPortId,
        status_event: PortStatusChanged,
    ) -> Result<Output<'b>, Error<<C as Controller>::BusError>> {
        let global_port_id = self
            .pd_controller
            .lookup_global_port(local_port_id)
            .map_err(Error::Pd)?;

        let status = controller.get_port_status(local_port_id).await?;
        trace!("Port{} status: {:#?}", global_port_id.0, status);

        let power = self.get_power_device(local_port_id).map_err(Error::Pd)?;
        trace!("Port{} status events: {:#?}", global_port_id.0, status_event);
        if status_event.plug_inserted_or_removed() {
            self.process_plug_event(controller, power, local_port_id, &status)
                .await?;
        }

        // Only notify power policy of a contract after Sink Ready event (always after explicit or implicit contract)
        if status_event.sink_ready() {
            self.process_new_consumer_contract(power, &status).await?;
        }

        if status_event.new_power_contract_as_provider() {
            self.process_new_provider_contract(power, &status).await?;
        }

        self.check_sink_ready_timeout(
            state,
            &status,
            local_port_id,
            status_event.new_power_contract_as_consumer(),
            status_event.sink_ready(),
        )
        .await?;

        Ok(Output::PortStatusChanged(OutputPortStatusChanged {
            port: local_port_id,
            status_event,
            status,
        }))
    }

    /// Finalize a port status change output
    async fn finalize_port_status_change(
        &self,
        state: &mut InternalState<N>,
        local_port: LocalPortId,
        status_event: PortStatusChanged,
        status: PortStatus,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        let global_port_id = self.pd_controller.lookup_global_port(local_port).map_err(Error::Pd)?;

        let port_index = local_port.0 as usize;
        let mut events = state.port_states[port_index].pending_events;
        events.status = events.status.union(status_event);
        state.port_states[port_index].pending_events = events;
        state.port_states[port_index].status = status;

        if events != PortEvent::none() {
            let mut pending = PortPending::none();
            pending.pend_port(global_port_id.0 as usize);
            self.pd_controller.notify_ports(pending).await;
            trace!("P{}: Notified service for events: {:#?}", global_port_id.0, events);
        }

        Ok(())
    }

    /// Finalize a PD alert output
    async fn finalize_pd_alert(
        &self,
        state: &mut InternalState<N>,
        local_port: LocalPortId,
        alert: Ado,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        let global_port_id = self.pd_controller.lookup_global_port(local_port).map_err(Error::Pd)?;
        let port_index = local_port.0 as usize;

        // Buffer the alert
        let backing = self.backing.lock().await;
        let channel = backing.pd_alert_channel(port_index).await.ok_or(PdError::InvalidPort)?;
        channel.0.publish_immediate(alert);

        // Pend the alert
        state.port_states[port_index]
            .pending_events
            .notification
            .set_alert(true);

        // Pend this port
        let mut pending = PortPending::none();
        pending.pend_port(global_port_id.0 as usize);
        self.pd_controller.notify_ports(pending).await;
        Ok(())
    }

    /// Wait for a pending port event
    ///
    /// DROP SAFETY: No state that needs to be restored
    async fn wait_port_pending(
        &self,
        controller: &mut C,
    ) -> Result<PortEventStreamer, Error<<C as Controller>::BusError>> {
        if self.state.lock().await.fw_update_state.in_progress() {
            // Don't process events while firmware update is in progress
            debug!("Firmware update in progress, ignoring port events");
            return pending().await;
        }

        let streaming_state = self.state.lock().await.port_event_streaming_state;
        if let Some(streamer) = streaming_state {
            // If we're converting the bitfields into an event stream yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            Ok(streamer)
        } else {
            // Otherwise, wait for the next port event
            // DROP SAFETY: Safe as long as `wait_port_event` is drop safe
            match select(controller.wait_port_event(), async {
                self.sw_status_event.wait().await;
                Ok::<_, Error<<C as Controller>::BusError>>(())
            })
            .await
            {
                Either::First(r) => r?,
                Either::Second(_) => (),
            };
            let pending: PortPending = FromIterator::from_iter(0..N);
            Ok(PortEventStreamer::new(pending.into_iter()))
        }
    }

    /// Wait for the next event
    pub async fn wait_next(&self) -> Result<Event<'_>, Error<<C as Controller>::BusError>> {
        // This loop is to ensure that if we finish streaming events we go back to waiting for the next port event
        loop {
            let event = {
                let mut controller = self.controller.lock().await;
                // DROP SAFETY: Select over drop safe functions
                select5(
                    self.wait_port_pending(&mut controller),
                    self.wait_power_command(),
                    self.pd_controller.receive(),
                    self.wait_cfu_command(),
                    self.wait_sink_ready_timeout(),
                )
                .await
            };
            match event {
                Either5::First(stream) => {
                    let mut stream = stream?;
                    if let Some((port_index, event)) = stream
                        .next::<Error<<C as Controller>::BusError>, _, _>(async |port_index| {
                            // Combine the event read from the controller with any software generated events
                            let sw_event: PortEvent =
                                self.state.lock().await.port_states[port_index].sw_status_event.into();
                            let hw_event = self
                                .controller
                                .lock()
                                .await
                                .clear_port_events(LocalPortId(port_index as u8))
                                .await?;
                            Ok(hw_event.union(sw_event))
                        })
                        .await?
                    {
                        let port_id = LocalPortId(port_index as u8);
                        self.state.lock().await.port_event_streaming_state = Some(stream);
                        match event {
                            PortEventVariant::StatusChanged(status_event) => {
                                return Ok(Event::PortStatusChanged(EventPortStatusChanged {
                                    port: port_id,
                                    status_event,
                                }));
                            }
                            PortEventVariant::Notification(notification) => {
                                return Ok(Event::PortNotification(EventPortNotification {
                                    port: port_id,
                                    notification,
                                }));
                            }
                        }
                    } else {
                        self.state.lock().await.port_event_streaming_state = None;
                    }
                }
                Either5::Second((port, request)) => {
                    return Ok(Event::PowerPolicyCommand(EventPowerPolicyCommand { port, request }))
                }
                Either5::Third(request) => return Ok(Event::ControllerCommand(request)),
                Either5::Fourth(event) => return Ok(Event::CfuEvent(event)),
                Either5::Fifth(port) => {
                    // Sink ready timeout event
                    debug!("Port{0}: Sink ready timeout", port.0);
                    self.state.lock().await.port_states[port.0 as usize].sink_ready_deadline = None;
                    let mut status_event = PortStatusChanged::none();
                    status_event.set_sink_ready(true);
                    return Ok(Event::PortStatusChanged(EventPortStatusChanged { port, status_event }));
                }
            }
        }
    }

    /// Process a port notification
    async fn process_port_notification<'b>(
        &self,
        controller: &mut C,
        port: LocalPortId,
        notification: PortNotificationSingle,
    ) -> Result<Output<'b>, Error<<C as Controller>::BusError>> {
        match notification {
            PortNotificationSingle::Alert => {
                let ado = controller.get_pd_alert(port).await?;
                trace!("Port{}: PD alert: {:#?}", port.0, ado);
                if let Some(ado) = ado {
                    Ok(Output::PdAlert(OutputPdAlert { port, ado }))
                } else {
                    // For some reason we didn't read an alert, nothing to do
                    Ok(Output::Nop)
                }
            }
            PortNotificationSingle::Vdm(event) => {
                self.process_vdm_event(controller, port, event).await.map(Output::Vdm)
            }
            PortNotificationSingle::DpStatusUpdate => self
                .process_dp_status_update(controller, port)
                .await
                .map(Output::DpStatusUpdate),
            rest => {
                // Nothing currently implemented for these
                trace!("Port{}: Notification: {:#?}", port.0, rest);
                Ok(Output::Nop)
            }
        }
    }

    /// Top-level processing function
    /// Only call this fn from one place in a loop. Otherwise a deadlock could occur.
    pub async fn process_event<'b>(&self, event: Event<'b>) -> Result<Output<'b>, Error<<C as Controller>::BusError>> {
        let mut controller = self.controller.lock().await;
        let mut state = self.state.lock().await;
        match event {
            Event::PortStatusChanged(EventPortStatusChanged { port, status_event }) => {
                self.process_port_status_changed(&mut controller, &mut state, port, status_event)
                    .await
            }
            Event::PowerPolicyCommand(EventPowerPolicyCommand { port, request }) => {
                let response = self
                    .process_power_command(&mut controller, &mut state, port, &request.command)
                    .await;
                Ok(Output::PowerPolicyCommand(OutputPowerPolicyCommand {
                    port,
                    request,
                    response,
                }))
            }
            Event::ControllerCommand(request) => {
                let response = self
                    .process_pd_command(&mut controller, &mut state, &request.command)
                    .await;
                Ok(Output::ControllerCommand(OutputControllerCommand { request, response }))
            }
            Event::CfuEvent(event) => match event {
                EventCfu::Request(request) => {
                    let response = self.process_cfu_command(&mut controller, &mut state, &request).await;
                    Ok(Output::CfuResponse(response))
                }
                EventCfu::RecoveryTick => {
                    // FW Update tick, process timeouts and recovery attempts
                    self.process_cfu_tick(&mut controller, &mut state).await;
                    Ok(Output::CfuRecovery)
                }
            },
            Event::PortNotification(EventPortNotification { port, notification }) => {
                self.process_port_notification(&mut controller, port, notification)
                    .await
            }
        }
    }

    /// Event loop finalize
    pub async fn finalize<'b>(&self, output: Output<'b>) -> Result<(), Error<<C as Controller>::BusError>> {
        let mut state = self.state.lock().await;

        match output {
            Output::Nop => Ok(()),
            Output::PortStatusChanged(OutputPortStatusChanged {
                port,
                status_event,
                status,
            }) => {
                self.finalize_port_status_change(&mut state, port, status_event, status)
                    .await
            }
            Output::PdAlert(OutputPdAlert { port, ado }) => self.finalize_pd_alert(&mut state, port, ado).await,
            Output::Vdm(vdm) => self.finalize_vdm(&mut state, vdm).await.map_err(Error::Pd),
            Output::PowerPolicyCommand(OutputPowerPolicyCommand { request, response, .. }) => {
                request.respond(response);
                Ok(())
            }
            Output::ControllerCommand(OutputControllerCommand { request, response }) => {
                request.respond(response);
                Ok(())
            }
            Output::CfuRecovery => {
                // Nothing to do here
                Ok(())
            }
            Output::CfuResponse(response) => {
                self.send_cfu_response(response).await;
                Ok(())
            }
            Output::DpStatusUpdate(_) => {
                // Nothing to do here
                Ok(())
            }
        }
    }

    /// Combined processing and finialization function
    pub async fn process_and_finalize_event<'b>(
        &self,
        event: Event<'b>,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        let output = self.process_event(event).await?;
        self.finalize(output).await
    }

    /// Combined processing function
    pub async fn process_next_event(&self) -> Result<(), Error<<C as Controller>::BusError>> {
        let event = self.wait_next().await?;
        self.process_and_finalize_event(event).await
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

impl<'a, const N: usize, C: Controller, BACK: Backing<'a>, V: FwOfferValidator> Object<C>
    for ControllerWrapper<'a, N, C, BACK, V>
{
    fn get_inner(&self) -> impl Future<Output = impl RefGuard<C>> {
        self.controller.lock()
    }

    fn get_inner_mut(&self) -> impl Future<Output = impl RefMutGuard<C>> {
        self.controller.lock()
    }
}
