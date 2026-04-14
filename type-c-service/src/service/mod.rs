use core::cell::RefCell;
use core::future::pending;
use core::pin::pin;

use embassy_futures::select::select_slice;
use embassy_futures::select::{Either, select};
use embedded_services::{debug, error, event::Receiver, info, trace};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::service::event::{PortEvent, PortEventData};

use type_c_interface::port::event::PortStatusEventBitfield;
use type_c_interface::port::{Device, PortStatus};
use type_c_interface::service::event;

pub mod config;
mod power;
mod ucsi;
pub mod vdm;

const MAX_SUPPORTED_PORTS: usize = 4;

/// Type-C service state
#[derive(Default)]
struct State {
    /// Current port status
    port_status: [PortStatus; MAX_SUPPORTED_PORTS],
    /// UCSI state
    ucsi: ucsi::State,
}

/// Type-C service
///
/// Constructing a Service is the first step in using the Type-C service.
/// Arguments should be an initialized context
pub struct Service<'a> {
    /// Type-C context
    pub(crate) context: &'a type_c_interface::service::context::Context,
    /// Current state
    state: State,
    /// Config
    config: config::Config,
}

/// Power policy events
// This is present instead of just using [`power_policy::CommsMessage`] to allow for
// supporting variants like `ConsumerConnected(GlobalPortId, ConsumerPowerCapability)`
// But there's currently not a way to do look-ups between power policy device IDs and GlobalPortIds
#[derive(Copy, Clone)]
pub enum PowerPolicyEvent {
    /// Unconstrained state changed
    Unconstrained(power_policy_interface::service::UnconstrainedState),
    /// Consumer disconnected
    ConsumerDisconnected,
    /// Consumer connected
    ConsumerConnected,
}

/// Type-C service events
#[derive(Copy, Clone)]
pub enum Event {
    /// Port event
    PortEvent(PortEvent),
    /// Power policy event
    PowerPolicy(PowerPolicyEvent),
}

impl<'a> Service<'a> {
    /// Create a new service the given configuration
    pub fn create(config: config::Config, context: &'a type_c_interface::service::context::Context) -> Self {
        Self {
            context,
            state: State::default(),
            config,
        }
    }

    /// Get the cached port status
    pub fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
        Ok(*self
            .state
            .port_status
            .get(port_id.0 as usize)
            .ok_or(Error::InvalidPort)?)
    }

    /// Set the cached port status
    fn set_cached_port_status(&mut self, port_id: GlobalPortId, status: PortStatus) -> Result<(), Error> {
        *self
            .state
            .port_status
            .get_mut(port_id.0 as usize)
            .ok_or(Error::InvalidPort)? = status;
        Ok(())
    }

    /// Process events for a specific port
    async fn process_port_status_event(
        &mut self,
        port_id: GlobalPortId,
        event: PortStatusEventBitfield,
        status: PortStatus,
    ) -> Result<(), Error> {
        let old_status = self.get_cached_port_status(port_id)?;

        debug!("Port{}: Event: {:#?}", port_id.0, event);
        debug!("Port{} Previous status: {:#?}", port_id.0, old_status);
        debug!("Port{} Status: {:#?}", port_id.0, status);

        let connection_changed = status.is_connected() != old_status.is_connected();
        if connection_changed && (status.is_debug_accessory() || old_status.is_debug_accessory()) {
            // Notify that a debug connection has connected/disconnected
            if status.is_connected() {
                debug!("Port{}: Debug accessory connected", port_id.0);
            } else {
                debug!("Port{}: Debug accessory disconnected", port_id.0);
            }

            self.context
                .broadcast_message(event::Event::DebugAccessory(event::DebugAccessory {
                    port: port_id,
                    connected: status.is_connected(),
                }))
                .await;
        }

        self.set_cached_port_status(port_id, status)?;
        self.handle_ucsi_port_event(port_id, event, &status).await;

        Ok(())
    }

    async fn process_port_event(&mut self, event: &PortEvent) -> Result<(), Error> {
        match &event.event {
            PortEventData::StatusChanged(status_event, status) => {
                self.process_port_status_event(event.port, *status_event, *status).await
            }
            unhandled => {
                // Currently just log notifications, but may want to do more in the future
                debug!("Port{}: Received notification event: {:#?}", event.port.0, unhandled);
                Ok(())
            }
        }
    }

    /// Process the given event
    pub async fn process_event(&mut self, event: Event) -> Result<(), Error> {
        match event {
            Event::PortEvent(event) => {
                trace!("Port{}: Processing port event", event.port.0);
                self.process_port_event(&event).await
            }
            Event::PowerPolicy(event) => {
                trace!("Processing power policy event");
                self.process_power_policy_event(&event).await
            }
        }
    }
}

/// Event receiver for the Type-C service
pub struct EventReceiver<'a, PowerReceiver: Receiver<PowerPolicyEventData>> {
    /// Type-C context
    pub(crate) context: &'a type_c_interface::service::context::Context,
    /// Power policy event subscriber
    ///
    /// Used to allow partial borrows of Self for the call to select
    power_policy_event_subscriber: RefCell<PowerReceiver>,
}

impl<'a, PowerReceiver: Receiver<PowerPolicyEventData>> EventReceiver<'a, PowerReceiver> {
    /// Create a new event receiver
    pub fn new(
        context: &'a type_c_interface::service::context::Context,
        power_policy_event_subscriber: PowerReceiver,
    ) -> Self {
        Self {
            context,
            power_policy_event_subscriber: RefCell::new(power_policy_event_subscriber),
        }
    }

    /// Wait for the next event
    pub async fn wait_next(&mut self) -> Event {
        match select(self.wait_port_event(), self.wait_power_policy_event()).await {
            Either::First(event) => event,
            Either::Second(event) => event,
        }
    }

    /// Wait for a port event
    async fn wait_port_event(&self) -> Event {
        let (event, _) = {
            let mut futures = heapless::Vec::<_, MAX_SUPPORTED_PORTS>::new();
            for device in self.context.controllers.iter_only::<Device>() {
                for descriptor in device.ports.iter() {
                    let _ = futures.push(async move { descriptor.receiver.receive().await });
                }
            }
            select_slice(pin!(&mut futures)).await
        };

        Event::PortEvent(event)
    }

    /// Wait for a power policy event
    #[allow(clippy::await_holding_refcell_ref)]
    async fn wait_power_policy_event(&self) -> Event {
        let Ok(mut subscriber) = self.power_policy_event_subscriber.try_borrow_mut() else {
            // This should never happen because this function is not public and is only called from wait_next, which takes &mut self
            error!("Attempt to call `wait_power_policy_event` simultaneously");
            return pending().await;
        };

        loop {
            match subscriber.wait_next().await {
                power_policy_interface::service::event::EventData::Unconstrained(state) => {
                    return Event::PowerPolicy(PowerPolicyEvent::Unconstrained(state));
                }
                power_policy_interface::service::event::EventData::ConsumerDisconnected => {
                    return Event::PowerPolicy(PowerPolicyEvent::ConsumerDisconnected);
                }
                power_policy_interface::service::event::EventData::ConsumerConnected(_) => {
                    return Event::PowerPolicy(PowerPolicyEvent::ConsumerConnected);
                }
                _ => {
                    // No other events currently implemented
                }
            }
        }
    }
}
