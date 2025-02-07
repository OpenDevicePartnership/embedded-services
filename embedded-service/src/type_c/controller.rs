//! PD controller related code
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{with_timeout, Duration};

use super::ucsi::lpm;
use super::{ControllerId, Error, PortId};
use crate::intrusive_list;

/// PD controller command-specific data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InternalCommandData {
    /// Reset the PD controller
    Reset,
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
pub type InternalResponse = Result<InternalResponseData, Error>;

/// PD controller command response
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    /// Controller response
    Controller(InternalResponse),
    /// UCSI response passthrough
    Lpm(lpm::Response),
}

/// PD controller
pub struct Controller<'a> {
    node: intrusive_list::Node,
    id: ControllerId,
    ports: &'a [PortId],
    command: Channel<NoopRawMutex, Command, 1>,
    response: Channel<NoopRawMutex, Response, 1>,
}

impl intrusive_list::NodeContainer for Controller<'static> {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

impl<'a> Controller<'a> {
    /// Create a new PD controller struct
    pub fn new(id: ControllerId, ports: &'a [PortId]) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            ports,
            command: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Send a command to this controller
    pub async fn send_command(&self, command: Command) -> Response {
        self.command.send(command).await;
        self.response.receive().await
    }

    /// Check if this controller has the given port
    pub fn has_port(&self, port: PortId) -> bool {
        self.ports.iter().any(|p| *p == port)
    }

    /// Wait for a command to be sent to this controller
    pub async fn wait_command(&self) -> Command {
        self.command.receive().await
    }

    /// Send response
    pub async fn send_response(&self, response: Response) {
        self.response.send(response).await;
    }
}

/// Trait for types that contain a controller struct
pub trait ControllerContainer {
    /// Get the controller struct
    fn get_controller<'a>(&'a self) -> &'a Controller<'a>;
}

/// Internal context for managing PD controllers
struct Context {
    controllers: intrusive_list::IntrusiveList,
}

impl Context {
    fn new() -> Self {
        Self {
            controllers: intrusive_list::IntrusiveList::new(),
        }
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Initialize the PD controller context
pub fn init() {
    CONTEXT.get_or_init(Context::new);
}

/// Register a PD controller
pub async fn register_controller(controller: &'static impl ControllerContainer) -> Result<(), intrusive_list::Error> {
    CONTEXT.get().await.controllers.push(controller.get_controller())
}

/// Default timeout for PD controller commands
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(250);

/// Send a command to the given controller with no timeout
async fn send_controller_command_no_timeout(
    controller_id: ControllerId,
    command: InternalCommandData,
) -> Result<InternalResponseData, Error> {
    let node = CONTEXT
        .get()
        .await
        .controllers
        .into_iter()
        .find(|node| {
            if let Some(controller) = node.data::<Controller>() {
                controller.id == controller_id
            } else {
                false
            }
        })
        .map_or(Error::InvalidController.into(), Ok)?;

    match node
        .data::<Controller>()
        .ok_or(Error::InvalidController)?
        .send_command(Command::Controller(command))
        .await
    {
        Response::Controller(response) => response,
        _ => Error::InvalidResponse.into(),
    }
}

/// Send a command to the given controller with a timeout
async fn send_controller_command(
    controller_id: ControllerId,
    command: InternalCommandData,
    timeout: Duration,
) -> Result<InternalResponseData, Error> {
    match with_timeout(timeout, send_controller_command_no_timeout(controller_id, command)).await {
        Ok(response) => response,
        Err(_) => Error::Timeout.into(),
    }
}

/// Reset the given controller
pub async fn reset_controller(controller_id: ControllerId) -> Result<(), Error> {
    send_controller_command(controller_id, InternalCommandData::Reset, DEFAULT_TIMEOUT)
        .await
        .map(|_| ())
}

/// Send a command to the given port
async fn send_port_command_no_timeout(port_id: PortId, command: lpm::CommandData) -> Result<lpm::ResponseData, Error> {
    let node = CONTEXT
        .get()
        .await
        .controllers
        .into_iter()
        .find(|node| {
            if let Some(controller) = node.data::<Controller>() {
                controller.has_port(port_id)
            } else {
                false
            }
        })
        .map_or(Error::InvalidPort.into(), Ok)?;

    match node
        .data::<Controller>()
        .ok_or(Error::InvalidController)?
        .send_command(Command::Lpm(lpm::Command {
            port: port_id,
            operation: command,
        }))
        .await
    {
        Response::Lpm(response) => response,
        _ => Error::InvalidResponse.into(),
    }
}

/// Send a command to the given port with a timeout
async fn send_port_command(
    port_id: PortId,
    command: lpm::CommandData,
    timeout: Duration,
) -> Result<lpm::ResponseData, Error> {
    match with_timeout(timeout, send_port_command_no_timeout(port_id, command)).await {
        Ok(response) => response,
        Err(_) => Error::Timeout.into(),
    }
}

/// Resets the given port
pub async fn reset_port(port_id: PortId, reset_type: lpm::ResetType) -> Result<lpm::ResponseData, Error> {
    send_port_command(port_id, lpm::CommandData::ConnectorReset(reset_type), DEFAULT_TIMEOUT).await
}
