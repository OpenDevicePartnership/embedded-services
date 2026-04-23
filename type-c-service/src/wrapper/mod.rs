//! This module contains the [`ControllerWrapper`] struct. This struct serves as a bridge between various service messages
//! and the actual controller functions provided by [`type_c_interface::port::Controller`].
//! # Supported service messaging
//! This struct currently supports messages from the following services:
//! * Type-C: [`type_c_interface::port::Command`]
//! * CFU: [`cfu_service::Request`]
//! # Event loop
//! This struct follows a standard process/finalize event loop.
//!
//! [`ControllerWrapper::process_event`] reads any additional data relevant to the event and returns [`message::Output`].
//! e.g. port status for a port status changed event, VDM data for a VDM event
//!
//! [`ControllerWrapper::finalize`] consumes [`message::Output`] and responds to any deferred requests, performs
//! any caching/buffering of data, and notifies the type-C service implementation of the event if needed.
use core::ops::DerefMut;

use crate::wrapper::backing::PortState;
use crate::wrapper::event_receiver::{ArrayPowerProxyEventReceiver, CfuEventReceiver, SinkReadyTimeoutEvent};
use cfu_service::CfuClient;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::event;
use embedded_services::sync::Lockable;
use embedded_services::{error, info, trace};
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::{Error, LocalPortId, PdError};
use type_c_interface::port::event::PortEvent as InterfacePortEvent;
use type_c_interface::service::event::{PortEvent as ServicePortEvent, PortEventData as ServicePortEventData};

use crate::wrapper::message::*;

pub mod backing;
mod cfu;
pub mod config;
mod dp;
pub mod event_receiver;
pub mod message;
mod pd;
mod power;
pub mod proxy;
mod vdm;

use type_c_interface::port::event::PortStatusEventBitfield;
use type_c_interface::port::{Controller, PortStatus};

/// Base interval for checking for FW update timeouts and recovery attempts
pub const DEFAULT_FW_UPDATE_TICK_INTERVAL_MS: u64 = 5000;
/// Default number of ticks before we consider a firmware update to have timed out
/// 300 seconds at 5 seconds per tick
pub const DEFAULT_FW_UPDATE_TIMEOUT_TICKS: u8 = 60;

/// Trait for validating firmware versions before applying an update
// TODO: remove this once we have a better framework for OEM customization
// See https://github.com/OpenDevicePartnership/embedded-services/issues/326
pub trait FwOfferValidator {
    /// Determine if we are accepting the firmware update offer, returns a CFU offer response
    fn validate(&self, current: FwVersion, offer: &FwUpdateOffer) -> FwUpdateOfferResponse;
}

/// Maximum number of supported ports
pub const MAX_SUPPORTED_PORTS: usize = 2;

/// Common functionality implemented on top of [`type_c_interface::port::Controller`]
pub struct ControllerWrapper<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: FwOfferValidator,
> where
    D::Inner: Controller,
{
    controller: &'device D,
    /// Trait object for validating firmware versions
    fw_version_validator: V,
    /// Registration information for services
    pub registration: backing::Registration<'device, M>,
    /// SW port status event signal
    sw_status_event: Signal<M, ()>,
    /// General config
    config: config::Config,
    /// Port proxies
    pub ports: &'device [backing::Port<'device, M, S>],
}

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, V>
where
    D::Inner: Controller,
{
    /// Create a new controller wrapper
    pub fn new<const N: usize>(
        controller: &'device D,
        config: config::Config,
        storage: &'device backing::ReferencedStorage<'device, N, M, S>,
        fw_version_validator: V,
    ) -> Self {
        const {
            assert!(N > 0 && N <= MAX_SUPPORTED_PORTS, "Invalid number of ports");
        };

        let backing = storage.create_backing();
        Self {
            controller,
            config,
            fw_version_validator,
            registration: backing.registration,
            sw_status_event: Signal::new(),
            ports: backing.ports,
        }
    }

    /// Get the cached port status, returns None if the port is invalid
    pub async fn get_cached_port_status(&self, local_port: LocalPortId) -> Option<PortStatus> {
        let port = self.ports.get(local_port.0 as usize)?;
        Some(port.state.lock().await.status)
    }

    /// Synchronize the state between the controller and the internal state
    pub async fn sync_state(&self) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        let mut controller = self.controller.lock().await;
        self.sync_state_internal(&mut controller).await
    }

    /// Synchronize the state between the controller and the internal state
    async fn sync_state_internal(
        &self,
        controller: &mut D::Inner,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        // Sync the controller state with the PD controller
        for (i, port) in self.ports.iter().enumerate() {
            let mut port_state = port.state.lock().await;

            let mut status_changed = port_state.sw_status_event;
            let local_port = LocalPortId(i as u8);
            let status = controller.get_port_status(local_port).await?;
            trace!("Port{} status: {:#?}", i, status);

            let previous_status = port_state.status;

            if previous_status.is_connected() != status.is_connected() {
                status_changed.set_plug_inserted_or_removed(true);
            }

            if previous_status.available_sink_contract != status.available_sink_contract {
                status_changed.set_new_power_contract_as_consumer(true);
            }

            if previous_status.available_source_contract != status.available_source_contract {
                status_changed.set_new_power_contract_as_provider(true);
            }

            port_state.sw_status_event = status_changed;
            if port_state.sw_status_event != PortStatusEventBitfield::none() {
                // Have a status changed event, notify
                trace!("Port{} status changed: {:#?}", i, status);
                self.sw_status_event.signal(());
            }
        }
        Ok(())
    }

    /// Handle a plug event
    async fn process_plug_event(
        &self,
        port_state: &mut PortState<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        info!("Plug event");
        if status.is_connected() {
            info!("Plug inserted");
            port_state
                .power_policy_sender
                .send(power_policy_interface::psu::event::EventData::Attached)
                .await;
        } else {
            info!("Plug removed");
            port_state
                .power_policy_sender
                .send(power_policy_interface::psu::event::EventData::Detached)
                .await;
        }

        Ok(())
    }

    /// Process port status changed events
    async fn process_port_status_changed<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        controller: &mut D::Inner,
        local_port_id: LocalPortId,
        status_event: PortStatusEventBitfield,
    ) -> Result<Output, Error<<D::Inner as Controller>::BusError>> {
        let global_port_id = self
            .registration
            .pd_controller
            .lookup_global_port(local_port_id)
            .map_err(Error::Pd)?;

        let mut port_state = self
            .ports
            .get(local_port_id.0 as usize)
            .ok_or(Error::Pd(PdError::InvalidPort))?
            .state
            .lock()
            .await;

        let status = controller.get_port_status(local_port_id).await?;
        trace!("Port{} status: {:#?}", global_port_id.0, status);
        trace!("Port{} status events: {:#?}", global_port_id.0, status_event);

        if status_event.plug_inserted_or_removed() {
            self.process_plug_event(&mut port_state, &status).await?;
        }

        // Only notify power policy of a contract after Sink Ready event (always after explicit or implicit contract)
        if status_event.sink_ready() {
            self.process_new_consumer_contract(&mut port_state, &status).await?;
        }

        if status_event.new_power_contract_as_provider() {
            self.process_new_provider_contract(&mut port_state, &status).await?;
        }

        self.check_sink_ready_timeout(
            sink_ready_timeout,
            &port_state.status,
            &status,
            local_port_id,
            status_event.new_power_contract_as_consumer(),
            status_event.sink_ready(),
        )?;

        Ok(Output::PortStatusChanged(OutputPortStatusChanged {
            port: local_port_id,
            status_event,
            status,
        }))
    }

    /// Finalize a port status change output
    async fn finalize_port_status_change(
        &self,
        local_port: LocalPortId,
        status_event: PortStatusEventBitfield,
        status: PortStatus,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        let global_port_id = self
            .registration
            .pd_controller
            .lookup_global_port(local_port)
            .map_err(Error::Pd)?;

        self.ports
            .get(local_port.0 as usize)
            .ok_or(Error::Pd(PdError::InvalidPort))?
            .state
            .lock()
            .await
            .status = status;

        self.registration
            .context
            .send_port_event(ServicePortEvent {
                port: global_port_id,
                event: ServicePortEventData::StatusChanged(status_event, status),
            })
            .await
            .map_err(Error::Pd)
    }

    /// Finalize a PD alert output
    async fn finalize_pd_alert(
        &self,
        local_port: LocalPortId,
        alert: Ado,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        let global_port_id = self
            .registration
            .pd_controller
            .lookup_global_port(local_port)
            .map_err(Error::Pd)?;

        self.registration
            .context
            .send_port_event(ServicePortEvent {
                port: global_port_id,
                event: ServicePortEventData::Alert(alert),
            })
            .await
            .map_err(Error::Pd)
    }

    /// Process a port notification
    async fn process_port_event<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        controller: &mut D::Inner,
        event: LocalPortEvent,
    ) -> Result<Output, Error<<D::Inner as Controller>::BusError>> {
        match event.event {
            InterfacePortEvent::StatusChanged(status_event) => {
                self.process_port_status_changed(sink_ready_timeout, controller, event.port, status_event)
                    .await
            }
            InterfacePortEvent::Alert => {
                let ado = controller.get_pd_alert(event.port).await?;
                trace!("Port{}: PD alert: {:#?}", event.port.0, ado);
                if let Some(ado) = ado {
                    Ok(Output::PdAlert(OutputPdAlert { port: event.port, ado }))
                } else {
                    // For some reason we didn't read an alert, nothing to do
                    Ok(Output::Nop)
                }
            }
            InterfacePortEvent::Vdm(vdm_event) => self
                .process_vdm_event(controller, event.port, vdm_event)
                .await
                .map(Output::Vdm),
            InterfacePortEvent::DpStatusUpdate => self
                .process_dp_status_update(controller, event.port)
                .await
                .map(Output::DpStatusUpdate),
            rest => {
                // Nothing currently implemented for these
                trace!("Port{}: Notification: {:#?}", event.port.0, rest);
                Ok(Output::Nop)
            }
        }
    }

    /// Top-level processing function
    /// Only call this fn from one place in a loop. Otherwise a deadlock could occur.
    pub async fn process_event<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        cfu_event_receiver: &mut CfuEventReceiver,
        event: Event,
    ) -> Result<Output, Error<<D::Inner as Controller>::BusError>> {
        let mut controller = self.controller.lock().await;
        match event {
            Event::PortEvent(port_event) => {
                self.process_port_event(sink_ready_timeout, &mut controller, port_event)
                    .await
            }
            Event::PowerPolicyCommand(PowerPolicyCommand { port, request }) => {
                let response = self
                    .process_power_command(cfu_event_receiver, &mut controller, port, &request)
                    .await;
                Ok(Output::PowerPolicyCommand(OutputPowerPolicyCommand { port, response }))
            }
            Event::CfuEvent(event) => match event {
                EventCfu::Request(request) => {
                    let response = self
                        .process_cfu_command(cfu_event_receiver, &mut controller, &request)
                        .await;
                    Ok(Output::CfuResponse(response))
                }
                EventCfu::RecoveryTick => {
                    // FW Update tick, process timeouts and recovery attempts
                    self.process_cfu_tick(cfu_event_receiver, &mut controller).await;
                    Ok(Output::CfuRecovery)
                }
            },
        }
    }

    /// Event loop finalize
    pub async fn finalize<const N: usize>(
        &self,
        event_receiver: &mut ArrayPowerProxyEventReceiver<'device, N>,
        output: Output,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        match output {
            Output::Nop => Ok(()),
            Output::PortStatusChanged(OutputPortStatusChanged {
                port,
                status_event,
                status,
            }) => self.finalize_port_status_change(port, status_event, status).await,
            Output::PdAlert(OutputPdAlert { port, ado }) => self.finalize_pd_alert(port, ado).await,
            Output::Vdm(vdm) => self.finalize_vdm(vdm).await.map_err(Error::Pd),
            Output::PowerPolicyCommand(OutputPowerPolicyCommand { port, response }) => {
                event_receiver
                    .send_response(port, response)
                    .await
                    .map_err(|_| Error::Pd(PdError::Failed))?;
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
    pub async fn process_and_finalize_event<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        cfu_event_receiver: &mut CfuEventReceiver,
        power_event_receiver: &mut ArrayPowerProxyEventReceiver<'device, N>,
        event: Event,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        let output = self
            .process_event(sink_ready_timeout, cfu_event_receiver, event)
            .await?;
        self.finalize(power_event_receiver, output).await
    }

    /// Register all devices with their respective services
    pub fn register(&'static self, cfu_client: &CfuClient) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        self.registration
            .context
            .register_controller(self.registration.pd_controller)
            .map_err(|_| {
                error!(
                    "Controller{}: Failed to register PD controller",
                    self.registration.pd_controller.id().0
                );
                Error::Pd(PdError::Failed)
            })?;

        //TODO: Remove when we have a more general framework in place
        cfu_client.register_device(self.registration.cfu_device).map_err(|_| {
            error!(
                "Controller{}: Failed to register CFU device",
                self.registration.pd_controller.id().0
            );
            Error::Pd(PdError::Failed)
        })?;
        Ok(())
    }
}

impl<
    'device,
    M: RawMutex,
    C: Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: FwOfferValidator,
> Lockable for ControllerWrapper<'device, M, C, S, V>
where
    <C as Lockable>::Inner: Controller,
{
    type Inner = C::Inner;

    fn try_lock(&self) -> Option<impl DerefMut<Target = Self::Inner>> {
        self.controller.try_lock()
    }

    fn lock(&self) -> impl Future<Output = impl DerefMut<Target = Self::Inner>> {
        self.controller.lock()
    }
}
