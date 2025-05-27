//! This module contains the `Controller` trait. Any types that implement this trait can be used with the `ControllerWrapper` struct
//! which provides a bridge between various service messages and the actual controller functions.
use core::array::from_fn;
use core::cell::{Cell, RefCell};

use embassy_futures::select::{select3, select_array, Either3};
use embedded_services::power::policy::device::StateKind;
use embedded_services::power::policy::{self, action};
use embedded_services::type_c::controller::{self, Controller, PortStatus};
use embedded_services::type_c::event::{PortEventFlags, PortEventKind};
use embedded_services::{error, info, trace, warn};
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId};

mod pd;
mod power;

/// Threshold power capability before we'll attempt to sink from a dual-role supply
/// This ensures we don't try to sink from something like a phone
const DUAL_ROLE_CONSUMER_THRESHOLD_MW: u32 = 15000;

/// Takes an implementation of the `Controller` trait and wraps it with logic to handle
/// message passing and power-policy integration.
pub struct ControllerWrapper<'a, const N: usize, C: Controller> {
    /// PD controller to interface with PD service
    pd_controller: controller::Device<'a>,
    /// Power policy devices to interface with power policy service
    power: [policy::device::Device; N],
    controller: RefCell<C>,
    active_events: [Cell<PortEventKind>; N],
}

impl<'a, const N: usize, C: Controller> ControllerWrapper<'a, N, C> {
    /// Create a new controller wrapper
    pub fn new(pd_controller: controller::Device<'a>, power: [policy::device::Device; N], controller: C) -> Self {
        Self {
            pd_controller,
            power,
            controller: RefCell::new(controller),
            active_events: [const { Cell::new(PortEventKind::none()) }; N],
        }
    }

    /// Ensure the software state is in sync with the hardware state
    #[allow(clippy::await_holding_refcell_ref)]
    async fn sync_state(&self) -> Result<(), Error<C::BusError>> {
        let mut controller = self.controller.borrow_mut();
        controller.sync_state().await
    }

    /// Handle a plug event
    async fn process_plug_event(
        &self,
        _controller: &mut C,
        power: &policy::device::Device,
        port: LocalPortId,
        status: &PortStatus,
    ) -> Result<(), Error<C::BusError>> {
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
    async fn process_event(&self, controller: &mut C) {
        let mut port_events = PortEventFlags::none();

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

            let status = match controller.get_port_status(local_port_id).await {
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

            if event.new_power_contract_as_consumer()
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
        }

        self.pd_controller.notify_ports(port_events).await;
    }

    /// Top-level processing function
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn process(&self) {
        let mut controller = self.controller.borrow_mut();
        match select3(
            controller.wait_port_event(),
            self.wait_power_command(),
            self.pd_controller.receive(),
        )
        .await
        {
            Either3::First(r) => match r {
                Ok(_) => self.process_event(&mut controller).await,
                Err(_) => error!("Error waiting for port event"),
            },
            Either3::Second((request, port)) => {
                let response = self
                    .process_power_command(&mut controller, port, &request.command)
                    .await;
                request.respond(response);
            }
            Either3::Third(request) => {
                let response = self.process_pd_command(&mut controller, &request.command).await;
                request.respond(response);
            }
        }
    }

    /// Register all devices with their respective services
    pub async fn register(&'static self) -> Result<(), Error<C::BusError>> {
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

        self.sync_state().await
    }
}
