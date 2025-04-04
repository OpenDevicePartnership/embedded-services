use core::cell::RefCell;
use embassy_futures::select::{select, Either};
use embassy_sync::once_lock::OnceLock;
use embedded_services::{
    comms::{self, EndpointID, Internal},
    debug, error, info,
    type_c::{
        self,
        controller::{ControllerStatus, PortStatus},
        event::PortEventFlags,
        external::{self, PortResponseData},
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
}

/// Type-C service
struct Service {
    /// Comms endpoint
    tp: comms::Endpoint,
    /// Type-C context token
    context: type_c::controller::ContextToken,
    /// Current state
    state: RefCell<State>,
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
    fn get_cached_port_status(&self, port_id: GlobalPortId) -> Result<PortStatus, Error> {
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
    async fn process_port_events(&self, port_id: GlobalPortId) -> Result<(), Error> {
        let event = self.context.get_port_event(port_id).await?;
        let status = self.context.get_port_status(port_id).await?;
        let old_status = self.get_cached_port_status(port_id)?;

        debug!("Port{}: Event: {:#?}", port_id.0, event);
        debug!("Port{} Previous status: {:#?}", port_id.0, old_status);
        debug!("Port{} Status: {:#?}", port_id.0, status);

        let connection_changed = status.connection_present != old_status.connection_present;
        if connection_changed && (status.debug_connection || old_status.debug_connection) {
            // Notify that a debug connection has connected/disconnected
            let msg = type_c::comms::DebugAccessoryMessage {
                port: port_id,
                connected: status.connection_present,
            };

            if status.connection_present {
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

    /// Process unhandled events
    async fn process_unhandled_events(&self, pending: PortEventFlags) {
        for i in 0..pending.len() {
            let port_id = GlobalPortId(i as u8);

            if !pending.is_pending(port_id) {
                continue;
            }

            debug!("Port{}: Event", i);
            if let Err(e) = self.process_port_events(port_id).await {
                error!("Port{}: Error processing events: {:#?}", i, e);
            }
        }
    }

    /// Process external controller commands
    async fn process_external_controller_command(&self, command: external::ControllerCommand) {
        debug!("Processing external controller command: {:#?}", command);
        // TODO: flesh this out
        self.context
            .send_external_response(external::Response::Controller(Ok(
                external::ControllerResponseData::ControllerStatus(ControllerStatus {
                    mode: "Normal",
                    valid_fw_bank: true,
                }),
            )))
            .await
    }

    /// Process external port commands
    async fn process_external_port_command(&self, command: external::PortCommand) {
        debug!("Processing external port command: {:#?}", command);
        // TODO: flesh this out
        self.context
            .send_external_response(external::Response::Port(Ok(PortResponseData::PortStatus(
                Default::default(),
            ))))
            .await
    }

    /// Process external commands
    async fn process_external_command(&self, command: external::Command) {
        match command {
            external::Command::Controller(command) => {
                self.process_external_controller_command(command).await;
            }
            external::Command::Port(command) => {
                self.process_external_port_command(command).await;
            }
        }
    }

    /// Main processing function
    pub async fn process(&self) {
        let message = select(
            self.context.get_unhandled_events(),
            self.context.wait_external_command(),
        )
        .await;
        match message {
            Either::First(pending) => self.process_unhandled_events(pending).await,
            Either::Second(command) => self.process_external_command(command).await,
        }
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, _message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Currently only need to send messages
        Ok(())
    }
}

#[embassy_executor::task]
pub async fn task() {
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

    if comms::register_endpoint(service, &service.tp).await.is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

    loop {
        service.process().await;
    }
}
