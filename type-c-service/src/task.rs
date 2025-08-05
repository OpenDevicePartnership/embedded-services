use core::future::Future;

use embassy_futures::select::{select3, Either3};
use embassy_sync::{
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel, WaitResult},
};
use embedded_services::{
    comms::{self, EndpointID, Internal},
    debug, error, info, intrusive_list,
    ipc::deferred,
    power::{self, policy::UnconstrainedState},
    trace,
    type_c::{
        self,
        controller::PortStatus,
        event::{PortNotificationSingle, PortStatusChanged},
        external::{self, ControllerCommandData},
        ControllerId,
    },
    GlobalRawMutex,
};
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;

use static_cell::StaticCell;

use crate::{PortEventStreamer, PortEventVariant};

const MAX_SUPPORTED_PORTS: usize = 4;

/// Type-C service state
#[derive(Default)]
struct State {
    /// Current port status
    port_status: [PortStatus; MAX_SUPPORTED_PORTS],
    /// Next port to check, this is used to round-robin through ports
    port_event_streaming_state: Option<PortEventStreamer>,
}

/// Maximum number of power policy events to buffer
pub const MAX_POWER_POLICY_EVENTS: usize = 4;

/// Type-C service
pub struct Service<'a> {
    /// Comms endpoint
    tp: comms::Endpoint,
    /// Type-C context token
    context: type_c::controller::ContextToken,
    /// Current state
    state: Mutex<GlobalRawMutex, State>,
    /// Power policy event receiver
    ///
    /// This is the corresponding publisher to [`Self::power_policy_event_subscriber`], power policy events
    /// will be buffered in the channel until they are brought into the event loop with the subscriber.
    power_policy_event_publisher: embedded_services::broadcaster::immediate::Receiver<'a, power::policy::CommsMessage>,
    /// Power policy event subscriber
    ///
    /// This is the corresponding subscriber to [`Self::power_policy_event_publisher`], needs to be a mutex because getting a message
    /// from the channel requires mutable access.
    power_policy_event_subscriber: Mutex<GlobalRawMutex, DynSubscriber<'a, power::policy::CommsMessage>>,
}

/// Power policy events
// This is present instead of just using [`power::policy::CommsMessage`] to allow for
// supporting variants like `ConsumerConnected(GlobalPortId, ConsumerPowerCapability)`
// But there's currently not a way to do look-ups between power policy device IDs and GlobalPortIds
pub enum PowerPolicyEvent {
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

/// Type-C service events
pub enum Event<'a> {
    /// Port event
    PortStatusChanged(GlobalPortId, PortStatusChanged, PortStatus),
    /// PD alert
    PdAlert(GlobalPortId, Ado),
    /// External command
    ExternalCommand(deferred::Request<'a, GlobalRawMutex, external::Command, external::Response<'static>>),
    /// Power policy event
    PowerPolicy(PowerPolicyEvent),
}

impl<'a> Service<'a> {
    /// Create a new service
    pub fn create(
        power_policy_publisher: DynImmediatePublisher<'a, power::policy::CommsMessage>,
        power_policy_subscriber: DynSubscriber<'a, power::policy::CommsMessage>,
    ) -> Option<Self> {
        Some(Self {
            tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Usbc)),
            context: type_c::controller::ContextToken::create()?,
            state: Mutex::new(State::default()),
            power_policy_event_publisher: power_policy_publisher.into(),
            power_policy_event_subscriber: Mutex::new(power_policy_subscriber),
        })
    }

    /// Get the cached port status
    pub async fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let state = self.state.lock().await;
        Ok(state.port_status[port_id.0 as usize])
    }

    /// Set the cached port status
    async fn set_cached_port_status(&self, port_id: GlobalPortId, status: PortStatus) -> Result<(), Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let mut state = self.state.lock().await;
        state.port_status[port_id.0 as usize] = status;
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
            let msg = type_c::comms::DebugAccessoryMessage {
                port: port_id,
                connected: status.is_connected(),
            };

            if status.is_connected() {
                debug!("Port{}: Debug accessory connected", port_id.0);
            } else {
                debug!("Port{}: Debug accessory disconnected", port_id.0);
            }

            if self.tp.send(EndpointID::Internal(Internal::Usbc), &msg).await.is_err() {
                error!("Failed to send debug accessory message");
            }
        }

        self.set_cached_port_status(port_id, status).await?;

        Ok(())
    }

    /// Process external controller status command
    async fn process_external_controller_status(&self, controller: ControllerId) -> external::Response<'static> {
        let status = self.context.get_controller_status(controller).await;
        if let Err(e) = status {
            error!("Error getting controller status: {:#?}", e);
        }
        external::Response::Controller(status.map(external::ControllerResponseData::ControllerStatus))
    }

    /// Process external controller sync state command
    async fn process_external_controller_sync_state(&self, controller: ControllerId) -> external::Response<'static> {
        let status = self.context.sync_controller_state(controller).await;
        if let Err(e) = status {
            error!("Error getting controller sync state: {:#?}", e);
        }
        external::Response::Controller(status.map(|_| external::ControllerResponseData::Complete))
    }

    /// Process external controller commands
    async fn process_external_controller_command(
        &self,
        command: &external::ControllerCommand,
    ) -> external::Response<'static> {
        debug!("Processing external controller command: {:#?}", command);
        match command.data {
            ControllerCommandData::ControllerStatus => self.process_external_controller_status(command.id).await,
            ControllerCommandData::SyncState => self.process_external_controller_sync_state(command.id).await,
        }
    }

    /// Process external port status command
    async fn process_external_port_status(&self, port_id: GlobalPortId, cached: bool) -> external::Response<'static> {
        let status = self.context.get_port_status(port_id, cached).await;
        if let Err(e) = status {
            error!("Error getting port status: {:#?}", e);
        }
        external::Response::Port(status.map(external::PortResponseData::PortStatus))
    }

    /// Process get retimer fw update status commands
    async fn process_get_rt_fw_update_status(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.get_rt_fw_update_status(port_id).await;
        if let Err(e) = status {
            error!("Error getting retimer fw update status: {:#?}", e);
        }

        external::Response::Port(status.map(external::PortResponseData::RetimerFwUpdateGetState))
    }

    /// Process set retimer fw update state commands
    async fn process_set_rt_fw_update_state(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.set_rt_fw_update_state(port_id).await;
        if let Err(e) = status {
            error!("Error setting retimer fw update state: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process clear retimer fw update state commands
    async fn process_clear_rt_fw_update_state(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.clear_rt_fw_update_state(port_id).await;
        if let Err(e) = status {
            error!("Error clear retimer fw update state: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process set retimer compliance
    async fn process_set_rt_compliance(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.set_rt_compliance(port_id).await;
        if let Err(e) = status {
            error!("Error set retimer compliance: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_reconfigure_retimer(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.reconfigure_retimer(port_id).await;
        if let Err(e) = status {
            error!("Error reconfiguring retimer: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_set_max_sink_voltage(
        &self,
        port_id: GlobalPortId,
        max_voltage_mv: Option<u16>,
    ) -> external::Response<'static> {
        let status = self.context.set_max_sink_voltage(port_id, max_voltage_mv).await;
        if let Err(e) = status {
            error!("Error setting max voltage: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_clear_dead_battery_flag(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.clear_dead_battery_flag(port_id).await;
        if let Err(e) = status {
            error!("Error clearing dead battery flag: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process external port commands
    async fn process_external_port_command(&self, command: &external::PortCommand) -> external::Response<'static> {
        debug!("Processing external port command: {:#?}", command);
        match command.data {
            external::PortCommandData::PortStatus(cached) => {
                self.process_external_port_status(command.port, cached).await
            }
            external::PortCommandData::RetimerFwUpdateGetState => {
                self.process_get_rt_fw_update_status(command.port).await
            }
            external::PortCommandData::RetimerFwUpdateSetState => {
                self.process_set_rt_fw_update_state(command.port).await
            }
            external::PortCommandData::RetimerFwUpdateClearState => {
                self.process_clear_rt_fw_update_state(command.port).await
            }
            external::PortCommandData::SetRetimerCompliance => self.process_set_rt_compliance(command.port).await,
            external::PortCommandData::ReconfigureRetimer => self.process_reconfigure_retimer(command.port).await,
            external::PortCommandData::SetMaxSinkVoltage { max_voltage_mv } => {
                self.process_set_max_sink_voltage(command.port, max_voltage_mv).await
            }
            external::PortCommandData::ClearDeadBatteryFlag => self.process_clear_dead_battery_flag(command.port).await,
        }
    }

    /// Process external commands
    async fn process_external_command(&self, command: &external::Command) -> external::Response<'static> {
        match command {
            external::Command::Controller(command) => self.process_external_controller_command(command).await,
            external::Command::Port(command) => self.process_external_port_command(command).await,
        }
    }

    /// Set the unconstrained state for all ports
    async fn set_unconstrained_all(&self, unconstrained: bool) -> Result<(), Error> {
        for port_index in 0..self.context.get_num_ports().await {
            self.context
                .set_unconstrained_power(GlobalPortId(port_index as u8), unconstrained)
                .await?;
        }
        Ok(())
    }

    /// Processed unconstrained state change
    async fn process_unconstrained_state_change(&self, unconstrained_state: &UnconstrainedState) -> Result<(), Error> {
        if unconstrained_state.unconstrained {
            let state = self.state.lock().await;

            if unconstrained_state.available > 1 {
                // There are multiple available unconstrained consumers, set all ports to unconstrained
                // TODO: determine if we need to consider if we need to consider
                // if the system would still be unconstrained if the current consumer disconnected
                // https://github.com/OpenDevicePartnership/embedded-services/issues/428
                info!("Setting all ports to unconstrained power, multiple consumers available");
                self.set_unconstrained_all(true).await?;
            } else {
                // Only one unconstrained device is present, see if that's one of our ports
                let num_ports = self.context.get_num_ports().await;
                let unconstrained_port = state
                    .port_status
                    .iter()
                    .take(num_ports)
                    .position(|status| status.available_sink_contract.is_some() && status.unconstrained_power);

                if let Some(unconstrained_index) = unconstrained_port {
                    // One of our ports is the unconstrained consumer
                    // If it switches to sourcing then the system will no longer be unconstrained
                    // So set that port to constrained and unconstrain all others
                    info!(
                        "Setting port{} to constrained, all others unconstrained",
                        unconstrained_index
                    );
                    for port_index in 0..num_ports {
                        self.context
                            .set_unconstrained_power(GlobalPortId(port_index as u8), port_index != unconstrained_index)
                            .await?;
                    }
                } else {
                    // The system is unconstrained, but not by one of our ports
                    // So we can set all ports to unconstrained
                    info!("Setting all ports to unconstrained power");
                    self.set_unconstrained_all(true).await?;
                }
            }
        } else {
            // No longer drawing from an unconstrained source, set all ports to constrained
            info!("Setting all ports to constrained power");
            self.set_unconstrained_all(false).await?;
        }

        Ok(())
    }

    /// Process power policy events
    async fn process_power_policy_event(&self, message: &PowerPolicyEvent) -> Result<(), Error> {
        match message {
            PowerPolicyEvent::Unconstrained(state) => self.process_unconstrained_state_change(state).await,
        }
    }

    /// Wait for port flags
    async fn wait_port_flags(&self) -> PortEventStreamer {
        if let Some(ref streamer) = self.state.lock().await.port_event_streaming_state {
            // If we have an existing iterator, return it
            // Yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            *streamer
        } else {
            // Wait for the next port event and create a streamer
            PortEventStreamer::new(self.context.get_unhandled_events().await.into_iter())
        }
    }

    /// Wait for a power policy event
    async fn wait_power_policy_event(&self) -> Event<'_> {
        loop {
            match self.power_policy_event_subscriber.lock().await.next_message().await {
                WaitResult::Lagged(lagged) => {
                    // Missed some messages, all we can do is log an error
                    error!("Power policy {} event(s) lagged", lagged);
                }
                WaitResult::Message(message) => match message.data {
                    power::policy::CommsData::Unconstrained(state) => {
                        return Event::PowerPolicy(PowerPolicyEvent::Unconstrained(state));
                    }
                    _ => {
                        // No other events currently implemented
                    }
                },
            }
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
                                let status = self.context.get_port_status(port_id, true).await?;
                                return Ok(Event::PortStatusChanged(port_id, status_event, status));
                            }
                            PortEventVariant::Notification(notification) => match notification {
                                PortNotificationSingle::Alert => {
                                    if let Some(ado) = self.context.get_pd_alert(port_id).await? {
                                        // Return a PD alert event
                                        return Ok(Event::PdAlert(port_id, ado));
                                    } else {
                                        // Didn't get an ADO, wait for next event
                                        continue;
                                    }
                                }
                                _ => {
                                    // Other notifications currently unimplemented
                                    trace!("Unimplemented port notification: {:?}", notification);
                                    continue;
                                }
                            },
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
            Event::PdAlert(port, alert) => {
                // Port notifications currently don't have any processing logic
                info!("Port{}: Got PD alert: {:?}", port.0, alert);
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

    /// Register the Type-C service with the comms endpoint
    pub async fn register_comms(&'static self) -> Result<(), intrusive_list::Error> {
        comms::register_endpoint(self, &self.tp).await?;
        power::policy::policy::register_message_receiver(&self.power_policy_event_publisher).await
    }
}

impl comms::MailboxDelegate for Service<'_> {
    fn receive(&self, _message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Currently only need to send messages
        Ok(())
    }
}

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<'a, Fut: Future<Output = ()>, F: Fn(&'a Service) -> Fut>(f: F) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power::policy::CommsMessage, MAX_POWER_POLICY_EVENTS, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let service = Service::create(power_policy_publisher, power_policy_subscriber);
    let service = match service {
        Some(service) => service,
        None => {
            error!("Type-C service already initialized");
            return;
        }
    };

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    if service.register_comms().await.is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

    loop {
        f(service).await;
    }
}

#[embassy_executor::task]
pub async fn task() {
    task_closure(|service: &Service| async {
        if let Err(e) = service.process_next_event().await {
            error!("Type-C service processing error: {:#?}", e);
        }
    })
    .await;
}
