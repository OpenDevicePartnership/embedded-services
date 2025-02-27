//! PD controller related code
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration};
use embedded_usb_pd::{PdError, PortId as LocalPortId};

use super::event::{PortEventFlags, PortEventKind};
use super::ucsi::lpm;
use super::{ControllerId, GlobalPortId};
use crate::power::policy;
use crate::{intrusive_list, trace, IntrusiveNode};

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

/// Port-specific command data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortCommandData {
    /// Get port status
    PortStatus,
    /// Get event flags
    GetEvent,
}

/// Port-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortCommand {
    /// Port ID
    pub port: GlobalPortId,
    /// Command data
    pub data: PortCommandData,
}

/// Port-specific response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortResponseData {
    /// Command completed with no error
    Complete,
    /// Port status
    PortStatus(PortStatus),
    /// Event
    Event(PortEventKind),
}

impl PortResponseData {
    /// Helper function to convert to a result
    pub fn complete_or_err(self) -> Result<(), PdError> {
        match self {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }
}

/// Port-specific command response
pub type PortResponse = Result<PortResponseData, PdError>;

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
    /// Port command
    Port(PortCommand),
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
    /// Port response
    Port(PortResponse),
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
        trace!("Notify ports: {:#x}", events.0);
        if events.0 == 0 {
            return;
        }

        let context = CONTEXT.get().await;

        context
            .port_events
            .signal(if let Some(flags) = context.port_events.try_take() {
                flags | events
            } else {
                events
            });
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
    port_events: Signal<NoopRawMutex, PortEventFlags>,
}

impl Context {
    fn new() -> Self {
        Self {
            controllers: intrusive_list::IntrusiveList::new(),
            port_events: Signal::new(),
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

    async fn find_node_by_port(&self, port_id: GlobalPortId) -> Result<&IntrusiveNode, PdError> {
        CONTEXT
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
            .ok_or(PdError::InvalidPort)
    }

    /// Send a command to the given port
    pub async fn send_port_command_ucsi_no_timeout(
        &self,
        port_id: GlobalPortId,
        command: lpm::CommandData,
    ) -> Result<lpm::ResponseData, PdError> {
        let node = self.find_node_by_port(port_id).await?;

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
    pub async fn send_port_command_ucsi(
        &self,
        port_id: GlobalPortId,
        command: lpm::CommandData,
        timeout: Duration,
    ) -> Result<lpm::ResponseData, PdError> {
        match with_timeout(timeout, self.send_port_command_ucsi_no_timeout(port_id, command)).await {
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
        self.send_port_command_ucsi(port_id, lpm::CommandData::ConnectorReset(reset_type), DEFAULT_TIMEOUT)
            .await
    }

    /// Send a command to the given port with no timeout
    pub async fn send_port_command_no_timeout(
        &self,
        port_id: GlobalPortId,
        command: PortCommandData,
    ) -> Result<PortResponseData, PdError> {
        let node = self.find_node_by_port(port_id).await?;

        match node
            .data::<Device>()
            .ok_or(PdError::InvalidController)?
            .send_command(Command::Port(PortCommand {
                port: port_id,
                data: command,
            }))
            .await
        {
            Response::Port(response) => response,
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Send a command to the given port with a timeout
    pub async fn send_port_command(
        &self,
        port_id: GlobalPortId,
        command: PortCommandData,
        timeout: Duration,
    ) -> Result<PortResponseData, PdError> {
        match with_timeout(timeout, self.send_port_command_no_timeout(port_id, command)).await {
            Ok(response) => response,
            Err(_) => Err(PdError::Timeout),
        }
    }

    /// Get the current port events
    pub async fn get_unhandled_events(&self) -> PortEventFlags {
        CONTEXT.get().await.port_events.wait().await
    }

    /// Get the unhandled events for the given port
    pub async fn get_port_event(&self, port: GlobalPortId) -> Result<PortEventKind, PdError> {
        match self
            .send_port_command(port, PortCommandData::GetEvent, DEFAULT_TIMEOUT)
            .await?
        {
            PortResponseData::Event(event) => Ok(event),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Get the current port status
    pub async fn get_port_status(&self, port: GlobalPortId) -> Result<PortStatus, PdError> {
        match self
            .send_port_command(port, PortCommandData::PortStatus, DEFAULT_TIMEOUT)
            .await?
        {
            PortResponseData::PortStatus(status) => Ok(status),
            _ => Err(PdError::InvalidResponse),
        }
    }
}
