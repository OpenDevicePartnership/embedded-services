use core::{cell::RefCell, future::Future};
use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, once_lock::OnceLock};
use embedded_services::{
    comms::{self, EndpointID, Internal},
    debug, error, info, intrusive_list,
    ipc::deferred,
    type_c::{
        self,
        controller::PortStatus,
        event::{PortEventFlagsIter, PortEventKind},
        external::{self, ControllerCommandData},
        ControllerId,
    },
};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;

const MAX_SUPPORTED_PORTS: usize = 4;

/// Type-C service state
#[derive(Default)]
struct State {
    /// Current port status
    port_status: [PortStatus; MAX_SUPPORTED_PORTS],
    /// Next port to check, this is used to round-robin through ports
    event_iter: Option<PortEventFlagsIter>,
}

/// Type-C service
pub struct Service {
    /// Comms endpoint
    tp: comms::Endpoint,
    /// Type-C context token
    context: type_c::controller::ContextToken,
    /// Current state
    state: RefCell<State>,
}

pub enum Event<'a> {
    /// Port event
    PortEvent(GlobalPortId, PortEventKind, PortStatus),
    /// External command
    ExternalCommand(deferred::Request<'a, NoopRawMutex, external::Command, external::Response<'static>>),
}

impl Service {
    /// Create a new service
    pub fn create() -> Option<Self> {
        Some(Self {
            tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Usbc)),
            context: type_c::controller::ContextToken::create()?,
            state: RefCell::new(State::default()),
        })
    }

    /// Get the cached port status
    pub fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let state = self.state.borrow();
        Ok(state.port_status[port_id.0 as usize])
    }

    /// Set the cached port status
    fn set_cached_port_status(&self, port_id: GlobalPortId, status: PortStatus) -> Result<(), Error> {
        if port_id.0 as usize >= MAX_SUPPORTED_PORTS {
            return Err(Error::InvalidPort);
        }

        let mut state = self.state.borrow_mut();
        state.port_status[port_id.0 as usize] = status;
        Ok(())
    }

    /// Process events for a specific port
    async fn process_port_event(
        &self,
        port_id: GlobalPortId,
        event: PortEventKind,
        status: PortStatus,
    ) -> Result<(), Error> {
        let old_status = self.get_cached_port_status(port_id)?;

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

        self.set_cached_port_status(port_id, status)?;

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
    async fn process_external_port_status(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.get_port_status(port_id).await;
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

    /// Process external port commands
    async fn process_external_port_command(&self, command: &external::PortCommand) -> external::Response<'static> {
        debug!("Processing external port command: {:#?}", command);
        match command.data {
            external::PortCommandData::PortStatus => self.process_external_port_status(command.port).await,
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
        }
    }

    /// Process external commands
    async fn process_external_command(&self, command: &external::Command) -> external::Response<'static> {
        match command {
            external::Command::Controller(command) => self.process_external_controller_command(command).await,
            external::Command::Port(command) => self.process_external_port_command(command).await,
        }
    }

    /// Wait for port flags
    #[allow(clippy::await_holding_refcell_ref)]
    async fn wait_port_flags(&self) -> PortEventFlagsIter {
        let mut state = self.state.borrow_mut();
        if state.event_iter.is_some() {
            // If we have an existing iterator, return it
            // Yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            state.event_iter.take().unwrap()
        } else {
            self.context.get_unhandled_events().await.into_iter()
        }
    }

    /// Wait for the next event
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn wait_next(&self) -> Result<Event<'_>, Error> {
        loop {
            match select(self.wait_port_flags(), self.context.wait_external_command()).await {
                Either::First(mut pending) => {
                    let mut state = self.state.borrow_mut();
                    if let Some(port_id) = pending.next() {
                        debug!("Port{}: Event", port_id.0);
                        state.event_iter = Some(pending);
                        let event = self.context.get_port_event(port_id).await?;
                        let status = self.context.get_port_status(port_id).await?;

                        return Ok(Event::PortEvent(port_id, event, status));
                    } else {
                        debug!("No pending event, continuing");
                        state.event_iter = None;
                        continue;
                    }
                }
                Either::Second(request) => {
                    return Ok(Event::ExternalCommand(request));
                }
            }
        }
    }

    /// Process the given event
    pub async fn process_event(&self, event: Event<'_>) -> Result<(), Error> {
        match event {
            Event::PortEvent(port, event_kind, status) => self.process_port_event(port, event_kind, status).await,
            Event::ExternalCommand(request) => {
                let response = self.process_external_command(&request.command).await;
                request.respond(response);
                Ok(())
            }
        }
    }

    /// Combined processing function
    pub async fn process(&self) -> Result<(), Error> {
        let event = self.wait_next().await?;
        self.process_event(event).await
    }

    /// Register the Type-C service with the comms endpoint
    pub async fn register_comms(&'static self) -> Result<(), intrusive_list::Error> {
        comms::register_endpoint(self, &self.tp).await
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, _message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Currently only need to send messages
        Ok(())
    }
}

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<'a, Fut: Future<Output = ()>, F: Fn(&'a Service) -> Fut>(f: F) {
    info!("Starting type-c task");

    let service = Service::create();
    let service = match service {
        Some(service) => service,
        None => {
            error!("Type-C service already initialized");
            return;
        }
    };

    static SERVICE: OnceLock<Service> = OnceLock::new();
    let service = SERVICE.get_or_init(|| service);

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
        if let Err(e) = service.process().await {
            error!("Type-C service processing error: {:#?}", e);
        }
    })
    .await;
}
