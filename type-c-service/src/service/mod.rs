use core::marker::PhantomData;
use core::ptr;

use embedded_services::event::Sender as _;
use embedded_services::named::Named as _;
use embedded_services::sync::Lockable;
use embedded_services::{debug, error, info, trace};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::control::pd::PortStatus;
use type_c_interface::port::pd::Pd;
use type_c_interface::service::event::{DebugAccessoryData, EventData, PortEvent, PortEventData};

use type_c_interface::port::event::PortStatusEventBitfield;
use type_c_interface::service::event::Event as ServiceEvent;

use crate::service::registration::Registration;

pub mod config;
pub mod event_receiver;
mod power;
pub mod registration;
mod ucsi;

/// Type-C service
///
/// Constructing a Service is the first step in using the Type-C service.
/// Arguments should be an initialized context
pub struct Service<'port, Reg: Registration<'port>> {
    /// UCSI state
    ucsi: ucsi::State,
    /// Config
    config: config::Config,
    /// Service registration
    registration: Reg,
    _phantom: PhantomData<&'port ()>,
}

/// Type-C service events
#[derive(Clone)]
pub enum Event<'port, Port: Lockable<Inner: Pd>> {
    /// Port event
    PortEvent(PortEvent<'port, Port>),
    /// Power policy event
    PowerPolicy(PowerPolicyEventData),
}

impl<'port, Reg: Registration<'port>> Service<'port, Reg> {
    /// Create a new service the given configuration
    pub fn create(config: config::Config, registration: Reg) -> Self {
        Self {
            ucsi: ucsi::State::default(),
            config,
            registration,
            _phantom: PhantomData,
        }
    }

    fn get_port_index(&self, port: &'port Reg::Port) -> Result<usize, Error> {
        self.registration
            .ports()
            .iter()
            .position(|p| ptr::eq(*p, port))
            .ok_or(Error::InvalidPort)
    }

    /// Look up the port for a given global port ID
    fn lookup_port(&self, port_id: GlobalPortId) -> Result<&'port Reg::Port, Error> {
        self.registration
            .ports()
            .get(port_id.0 as usize)
            .ok_or(Error::InvalidPort)
            .copied()
    }

    /// Send an event to all registered listeners
    async fn broadcast_event(&mut self, event: ServiceEvent<'port, Reg::Port>) {
        for sender in self.registration.event_senders() {
            sender.send(event.clone()).await;
        }
    }

    /// Process events for a specific port
    async fn process_port_status_event(
        &mut self,
        port: &'port Reg::Port,
        event: PortStatusEventBitfield,
        new_status: PortStatus,
        old_status: PortStatus,
    ) -> Result<(), Error> {
        let port_name = { port.lock().await.name() };

        debug!("({}): Event: {:#?}", port_name, event);
        debug!("({}) Previous status: {:#?}", port_name, old_status);
        debug!("({}) Status: {:#?}", port_name, new_status);

        let connection_changed = new_status.is_connected() != old_status.is_connected();
        if connection_changed && (new_status.is_debug_accessory() || old_status.is_debug_accessory()) {
            // Notify that a debug connection has connected/disconnected
            if new_status.is_connected() {
                debug!("({}): Debug accessory connected", port_name);
            } else {
                debug!("({}): Debug accessory disconnected", port_name);
            }

            self.broadcast_event(ServiceEvent {
                port,
                event: EventData::DebugAccessory(DebugAccessoryData {
                    connected: new_status.is_connected(),
                }),
            })
            .await;
        }

        self.handle_ucsi_port_event(port, GlobalPortId(self.get_port_index(port)? as u8), event, &new_status)
            .await;

        Ok(())
    }

    async fn process_port_event(&mut self, event: &PortEvent<'port, Reg::Port>) -> Result<(), Error> {
        match &event.event {
            PortEventData::StatusChanged(status_event) => {
                self.process_port_status_event(
                    event.port,
                    status_event.status_event,
                    status_event.current_status,
                    status_event.previous_status,
                )
                .await
            }
            unhandled => {
                // Currently just log notifications, but may want to do more in the future
                debug!(
                    "({}): Received notification event: {:#?}",
                    event.port.lock().await.name(),
                    unhandled
                );
                Ok(())
            }
        }
    }

    /// Process the given event
    pub async fn process_event(&mut self, event: Event<'port, Reg::Port>) -> Result<(), Error> {
        match event {
            Event::PortEvent(event) => {
                trace!("({}): Processing port event", event.port.lock().await.name());
                self.process_port_event(&event).await
            }
            Event::PowerPolicy(event) => {
                trace!("Processing power policy event");
                self.process_power_policy_event(&event).await
            }
        }
    }
}
