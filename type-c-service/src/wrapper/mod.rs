//! This module contains the `Controller` trait. Any types that implement this trait can be used with the `ControllerWrapper` struct
//! which provides a bridge between various service messages and the actual controller functions.
use core::array::from_fn;
use core::cell::{Cell, RefCell};
use core::future::Future;

use bitfield::BitMut;
use embassy_futures::select::{select3, select_array, Either3};
use embedded_services::power::policy::device::{RequestData, StateKind};
use embedded_services::power::policy::{self, action};
use embedded_services::type_c::controller::{self, Contract, PortStatus};
use embedded_services::type_c::event::{PortEventFlags, PortEventKind};
use embedded_services::{error, info, intrusive_list, trace, warn};
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId};

pub mod pd;
pub mod power;

/// PD controller trait for use with wrapper struct
pub trait Controller {
    type BusError;

    /// Returns ports with pending events
    fn wait_port_event(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Returns and clears current events for the given port
    fn clear_port_events(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<PortEventKind, Error<Self::BusError>>>;
    /// Returns the port status
    fn get_port_status(&mut self, port: LocalPortId)
        -> impl Future<Output = Result<PortStatus, Error<Self::BusError>>>;
    /// Enable or disable sink path
    fn enable_sink_path(
        &mut self,
        port: LocalPortId,
        enable: bool,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
}

/// Takes an implementation of the `Controller` trait and wraps it with logic to handle
/// message passing and power-policy integration.
pub struct ControllerWrapper<const N: usize, C: Controller> {
    /// PD controller to interface with PD service
    pd_controller: controller::Device,
    /// Power policy devices to interface with power policy service
    power: [policy::device::Device; N],
    controller: RefCell<C>,
    active_events: [Cell<PortEventKind>; N],
}

impl<const N: usize, C: Controller> ControllerWrapper<N, C> {
    /// Create a new controller wrapper
    pub fn new(pd_controller: controller::Device, power: [policy::device::Device; N], controller: C) -> Self {
        Self {
            pd_controller,
            power,
            controller: RefCell::new(controller),
            active_events: [const { Cell::new(PortEventKind::NONE) }; N],
        }
    }

    /// Handle a plug event
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    async fn process_plug_event(&self, power: &policy::device::Device, status: &PortStatus) {
        info!("Plug event");

        if status.connection_present {
            info!("Plug inserted");

            // Recover if we're not in the correct state
            if power.state().await.kind() != StateKind::Detached {
                warn!("Power device not in detached state, recovering");
                if let Err(e) = power.detach().await {
                    error!("Error detaching power device: {:?}", e);
                    return;
                }
            }

            if let Ok(state) = power.try_device_action::<action::Detached>().await {
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return;
                }
            } else {
                // This should never happen
                error!("Power device not in detached state");
                return;
            }
        } else {
            info!("Plug removed");
            if let Err(e) = power.detach().await {
                error!("Error detaching power device: {:?}", e);
                return;
            };
        }
    }

    /// Process port events
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    async fn process_event(&self, controller: &mut C) {
        let mut port_events = PortEventFlags(0);

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

            if event == PortEventKind::NONE {
                self.active_events[port].set(PortEventKind::NONE);
                continue;
            }

            port_events.set_bit(global_port_id.0.into(), true);

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
            if event.plug_inserted_or_removed() {
                self.process_plug_event(power, &status).await;
            }

            if event.new_power_contract_as_consumer() {
                self.process_new_consumer_contract(power, &status).await;
            }

            self.active_events[port].set(event);
        }

        self.pd_controller.notify_ports(port_events).await;
    }

    /// Top-level processing function
    pub async fn process(&self) {
        let mut controller = self.controller.borrow_mut();
        match select3(
            controller.wait_port_event(),
            self.wait_power_command(),
            self.pd_controller.wait_command(),
        )
        .await
        {
            Either3::First(r) => match r {
                Ok(_) => self.process_event(&mut controller).await,
                Err(_) => error!("Error waiting for port event"),
            },
            Either3::Second((command, port)) => self.process_power_command(&mut controller, port, command).await,
            Either3::Third(command) => self.process_pd_command(&mut controller, command).await,
        }
    }

    /// Register all devices with their respective services
    pub async fn register(&'static self) -> Result<(), intrusive_list::Error> {
        for device in &self.power {
            policy::register_device(device).await?
        }

        controller::register_controller(&self.pd_controller).await
    }
}
