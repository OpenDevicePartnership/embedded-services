//! PD controller related code
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{with_timeout, Duration};
use embedded_usb_pd::{PdError, PortId as LocalPortId};

use super::event::PortEventFlags;
use super::ucsi::lpm;
use super::{ControllerId, GlobalPortId};
use crate::intrusive_list;
use crate::power::policy;

/// Power contract
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Contract {
    /// Contract as sink
    Sink(policy::PowerCapability),
    /// Constract as source
    Source(policy::PowerCapability),
}

/// Port status
#[derive(Copy, Clone, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatus {
    /// Current power contract
    pub contract: Option<Contract>,
    /// Connection present
    pub connection_present: bool,
    /// Debug connection
    pub debug_connection: bool,
}

/// PD controller command-specific data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InternalCommandData {
    /// Reset the PD controller
    Reset,
    /// Acknowledge a port event
    AckEvent,
    /// Get port status
    PortStatus,
}

/// PD controller command
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    /// Controller specific command
    Controller(InternalCommandData),
    /// UCSI command passthrough
    Lpm(lpm::Command),
}

/// Controller-specific response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InternalResponseData {
    /// Command complete
    Complete,
}

/// Response for controller-specific commands
pub type InternalResponse = Result<InternalResponseData, PdError>;

/// PD controller command response
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    /// Controller response
    Controller(InternalResponse),
    /// UCSI response passthrough
    Lpm(lpm::Response),
}

/// Maximum number of controller ports
pub const MAX_CONTROLLER_PORTS: usize = 2;

/// PD controller
pub struct Device {
    node: intrusive_list::Node,
    id: ControllerId,
    ports: [GlobalPortId; MAX_CONTROLLER_PORTS],
    num_ports: usize,
    command: Channel<NoopRawMutex, Command, 1>,
    response: Channel<NoopRawMutex, Response, 1>,
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

impl Device {
    /// Create a new PD controller struct
    pub fn new(id: ControllerId, ports: &[GlobalPortId]) -> Result<Self, PdError> {
        Ok(Self {
            node: intrusive_list::Node::uninit(),
            id,
            ports: ports.try_into().map_err(|_| PdError::InvalidParams)?,
            num_ports: ports.len(),
            command: Channel::new(),
            response: Channel::new(),
        })
    }

    /// Send a command to this controller
    pub async fn send_command(&self, command: Command) -> Response {
        self.command.send(command).await;
        self.response.receive().await
    }

    /// Check if this controller has the given port
    pub fn has_port(&self, port: GlobalPortId) -> bool {
        self.ports.iter().any(|p| *p == port)
    }

    /// Covert a local port ID to a global port ID
    pub fn lookup_global_port(&self, port: LocalPortId) -> Result<GlobalPortId, PdError> {
        if port.0 >= self.num_ports as u8 {
            return Err(PdError::InvalidParams);
        }

        Ok(self.ports[port.0 as usize])
    }

    /// Wait for a command to be sent to this controller
    pub async fn wait_command(&self) -> Command {
        self.command.receive().await
    }

    /// Send response
    pub async fn send_response(&self, response: Response) {
        self.response.send(response).await;
    }

    /// Notify of a port event
    pub async fn notify_ports(&self, events: PortEventFlags) {
        let context = CONTEXT.get().await;
        context.port_events.set(context.port_events.get() | events);
    }

    /// Number of ports on this controller
    pub fn num_ports(&self) -> usize {
        self.num_ports
    }
}

/// Trait for types that contain a controller struct
pub trait DeviceContainer {
    /// Get the controller struct
    fn get_pd_controller_device<'a>(&'a self) -> &'a Device;
}

impl DeviceContainer for Device {
    fn get_pd_controller_device<'a>(&'a self) -> &'a Device {
        self
    }
}

/// Internal context for managing PD controllers
struct Context {
    controllers: intrusive_list::IntrusiveList,
    port_events: Cell<PortEventFlags>,
}

impl Context {
    fn new() -> Self {
        Self {
            controllers: intrusive_list::IntrusiveList::new(),
            port_events: Cell::new(PortEventFlags(0)),
        }
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Initialize the PD controller context
pub fn init() {
    CONTEXT.get_or_init(Context::new);
}

/// Register a PD controller
pub async fn register_controller(controller: &'static impl DeviceContainer) -> Result<(), intrusive_list::Error> {
    CONTEXT
        .get()
        .await
        .controllers
        .push(controller.get_pd_controller_device())
}

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(250);

/// Type to provide exclusive access to the PD controller context
pub struct ContextToken(());

impl ContextToken {
    /// Create a new context token, returning None if this function has been called before
    pub fn create() -> Option<Self> {
        static INIT: AtomicBool = AtomicBool::new(false);
        if INIT.load(Ordering::SeqCst) {
            return None;
        }

        INIT.store(true, Ordering::SeqCst);
        Some(ContextToken(()))
    }

    /// Send a command to the given controller with no timeout
    pub async fn send_controller_command_no_timeout(
        &self,
        controller_id: ControllerId,
        command: InternalCommandData,
    ) -> Result<InternalResponseData, PdError> {
        let node = CONTEXT
            .get()
            .await
            .controllers
            .into_iter()
            .find(|node| {
                if let Some(controller) = node.data::<Device>() {
                    controller.id == controller_id
                } else {
                    false
                }
            })
            .map_or(Err(PdError::InvalidController), Ok)?;

        match node
            .data::<Device>()
            .ok_or(PdError::InvalidController)?
            .send_command(Command::Controller(command))
            .await
        {
            Response::Controller(response) => response,
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Send a command to the given controller with a timeout
    pub async fn send_controller_command(
        &self,
        controller_id: ControllerId,
        command: InternalCommandData,
        timeout: Duration,
    ) -> Result<InternalResponseData, PdError> {
        match with_timeout(timeout, self.send_controller_command_no_timeout(controller_id, command)).await {
            Ok(response) => response,
            Err(_) => Err(PdError::Timeout),
        }
    }

    /// Reset the given controller
    pub async fn reset_controller(&self, controller_id: ControllerId) -> Result<(), PdError> {
        self.send_controller_command(controller_id, InternalCommandData::Reset, DEFAULT_TIMEOUT)
            .await
            .map(|_| ())
    }

    /// Send a command to the given port
    pub async fn send_port_command_no_timeout(
        &self,
        port_id: GlobalPortId,
        command: lpm::CommandData,
    ) -> Result<lpm::ResponseData, PdError> {
        let node = CONTEXT
            .get()
            .await
            .controllers
            .into_iter()
            .find(|node| {
                if let Some(controller) = node.data::<Device>() {
                    controller.has_port(port_id)
                } else {
                    false
                }
            })
            .map_or(Err(PdError::InvalidPort), Ok)?;

        match node
            .data::<Device>()
            .ok_or(PdError::InvalidController)?
            .send_command(Command::Lpm(lpm::Command {
                port: port_id,
                operation: command,
            }))
            .await
        {
            Response::Lpm(response) => response,
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Send a command to the given port with a timeout
    pub async fn send_port_command(
        &self,
        port_id: GlobalPortId,
        command: lpm::CommandData,
        timeout: Duration,
    ) -> Result<lpm::ResponseData, PdError> {
        match with_timeout(timeout, self.send_port_command_no_timeout(port_id, command)).await {
            Ok(response) => response,
            Err(_) => Err(PdError::Timeout),
        }
    }

    /// Resets the given port
    pub async fn reset_port(
        &self,
        port_id: GlobalPortId,
        reset_type: lpm::ResetType,
    ) -> Result<lpm::ResponseData, PdError> {
        self.send_port_command(port_id, lpm::CommandData::ConnectorReset(reset_type), DEFAULT_TIMEOUT)
            .await
    }
}
