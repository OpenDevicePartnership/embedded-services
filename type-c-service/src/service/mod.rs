use embassy_futures::select::{Either3, select3};
use embassy_sync::{
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber},
};
use embedded_services::{
    GlobalRawMutex, debug, error, info, intrusive_list,
    ipc::deferred,
    trace,
    type_c::{
        self, comms,
        controller::PortStatus,
        event::{PortNotificationSingle, PortStatusChanged},
        external,
    },
};
use embedded_services::{power::policy as power_policy, type_c::Cached};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;

use crate::{PortEventStreamer, PortEventVariant};

pub mod config;
mod controller;
pub mod pd;
mod port;
mod power;
mod ucsi;
pub mod vdm;

const MAX_SUPPORTED_PORTS: usize = 4;

/// Maximum number of power policy events to buffer
/// Arbitrary number, but power policy events in general shouldn't be too frequent
pub const MAX_POWER_POLICY_EVENTS: usize = 4;

/// Type-C service state
#[derive(Default)]
struct State {
    /// Current port status
    port_status: [PortStatus; MAX_SUPPORTED_PORTS],
    /// Next port to check, this is used to round-robin through ports
    port_event_streaming_state: Option<PortEventStreamer>,
    /// UCSI state
    ucsi: ucsi::State,
}

/// Type-C service
pub struct Service<'a> {
    /// Type-C context token
    context: type_c::controller::ContextToken,
    /// Current state
    state: Mutex<GlobalRawMutex, State>,
    /// Config
    config: config::Config,
    /// Power policy event receiver
    ///
    /// This is the corresponding publisher to [`Self::power_policy_event_subscriber`], power policy events
    /// will be buffered in the channel until they are brought into the event loop with the subscriber.
    power_policy_event_publisher: embedded_services::broadcaster::immediate::Receiver<'a, power_policy::CommsMessage>,
    /// Power policy event subscriber
    ///
    /// This is the corresponding subscriber to [`Self::power_policy_event_publisher`], needs to be a mutex because getting a message
    /// from the channel requires mutable access.
    power_policy_event_subscriber: Mutex<GlobalRawMutex, DynSubscriber<'a, power_policy::CommsMessage>>,
}

/// Power policy events
// This is present instead of just using [`power_policy::CommsMessage`] to allow for
// supporting variants like `ConsumerConnected(GlobalPortId, ConsumerPowerCapability)`
// But there's currently not a way to do look-ups between power policy device IDs and GlobalPortIds
pub enum PowerPolicyEvent {
    /// Unconstrained state changed
    Unconstrained(power_policy::UnconstrainedState),
    /// Consumer disconnected
    ConsumerDisconnected,
    /// Consumer connected
    ConsumerConnected,
}

/// Type-C service events
pub enum Event<'a> {
    /// Port event
    PortStatusChanged(GlobalPortId, PortStatusChanged, PortStatus),
    /// A controller notified of an event that occurred.
    PortNotification(GlobalPortId, PortNotificationSingle),
    /// External command
    ExternalCommand(deferred::Request<'a, GlobalRawMutex, external::Command, external::Response<'static>>),
    /// Power policy event
    PowerPolicy(PowerPolicyEvent),
}

impl<'a> Service<'a> {
    /// Create a new service the given configuration
    pub fn create(
        config: config::Config,
        power_policy_publisher: DynImmediatePublisher<'a, power_policy::CommsMessage>,
        power_policy_subscriber: DynSubscriber<'a, power_policy::CommsMessage>,
    ) -> Option<Self> {
        Some(Self {
            context: type_c::controller::ContextToken::create()?,
            state: Mutex::new(State::default()),
            config,
            power_policy_event_publisher: power_policy_publisher.into(),
            power_policy_event_subscriber: Mutex::new(power_policy_subscriber),
        })
    }

    /// Get the cached port status
    pub async fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
        let state = self.state.lock().await;
        Ok(*state.port_status.get(port_id.0 as usize).ok_or(Error::InvalidPort)?)
    }

    /// Set the cached port status
    async fn set_cached_port_status(&self, port_id: GlobalPortId, status: PortStatus) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        *state
            .port_status
            .get_mut(port_id.0 as usize)
            .ok_or(Error::InvalidPort)? = status;
        Ok(())
    }

    /// Process events for a specific port
    async fn process_port_event(
        &self,
        port_id: GlobalPortId,
        event: PortStatusChanged,
        status: PortStatus,
    ) -> Result<(), Error> {
        let old_status = self.get_cached_port_status(port_id).await?;

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
                .broadcast_message(comms::CommsMessage::DebugAccessory(comms::DebugAccessoryMessage {
                    port: port_id,
                    connected: status.is_connected(),
                }))
                .await;
        }

        self.set_cached_port_status(port_id, status).await?;
        self.handle_ucsi_port_event(port_id, event, &status).await;

        Ok(())
    }

    /// Process external commands
    async fn process_external_command(&self, command: &external::Command) -> external::Response<'static> {
        match command {
            external::Command::Controller(command) => self.process_external_controller_command(command).await,
            external::Command::Port(command) => self.process_external_port_command(command).await,
            external::Command::Ucsi(command) => external::Response::Ucsi(self.process_ucsi_command(command).await),
        }
    }

    /// Wait for the next event
    pub async fn wait_next(&self) -> Result<Event<'_>, Error> {
        loop {
            match select3(
                self.wait_port_flags(),
                self.context.wait_external_command(),
                self.wait_power_policy_event(),
            )
            .await
            {
                Either3::First(mut stream) => {
                    if let Some((port_id, event)) = stream
                        .next(|port_id| self.context.get_port_event(GlobalPortId(port_id as u8)))
                        .await?
                    {
                        let port_id = GlobalPortId(port_id as u8);
                        self.state.lock().await.port_event_streaming_state = Some(stream);
                        match event {
                            PortEventVariant::StatusChanged(status_event) => {
                                // Return a port status changed event
                                let status = self.context.get_port_status(port_id, Cached(true)).await?;
                                return Ok(Event::PortStatusChanged(port_id, status_event, status));
                            }
                            PortEventVariant::Notification(notification) => {
                                // Other notifications
                                trace!("Port notification: {:?}", notification);
                                return Ok(Event::PortNotification(port_id, notification));
                            }
                        }
                    } else {
                        self.state.lock().await.port_event_streaming_state = None;
                    }
                }
                Either3::Second(request) => {
                    return Ok(Event::ExternalCommand(request));
                }
                Either3::Third(event) => return Ok(event),
            }
        }
    }

    /// Process the given event
    pub async fn process_event(&self, event: Event<'_>) -> Result<(), Error> {
        match event {
            Event::PortStatusChanged(port, event_kind, status) => {
                trace!("Port{}: Processing port status changed", port.0);
                self.process_port_event(port, event_kind, status).await
            }
            Event::PortNotification(port, notification) => {
                // Other port notifications
                info!("Port{}: Got port notification: {:?}", port.0, notification);
                Ok(())
            }
            Event::ExternalCommand(request) => {
                trace!("Processing external command");
                let response = self.process_external_command(&request.command).await;
                request.respond(response);
                Ok(())
            }
            Event::PowerPolicy(event) => {
                trace!("Processing power policy event");
                self.process_power_policy_event(&event).await
            }
        }
    }

    /// Combined processing function
    pub async fn process_next_event(&self) -> Result<(), Error> {
        let event = self.wait_next().await?;
        self.process_event(event).await
    }

    /// Register the Type-C service with the power policy service
    pub fn register_comms(&'static self) -> Result<(), intrusive_list::Error> {
        power_policy::policy::register_message_receiver(&self.power_policy_event_publisher)
    }
}
