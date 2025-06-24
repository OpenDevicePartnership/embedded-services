//! PD controller related code
use core::future::Future;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration};
use embedded_usb_pd::ucsi::lpm;
use embedded_usb_pd::{
    pdinfo::{AltMode, PowerPathStatus},
    type_c::ConnectionState,
    Error, GlobalPortId, PdError, PortId as LocalPortId,
};

use super::event::{PortEventFlags, PortEventKind};
use super::{external, ControllerId};
use crate::ipc::deferred;
use crate::power::policy;
use crate::{error, intrusive_list, trace, IntrusiveNode};

/// Power contract
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Contract {
    /// Contract as sink
    Sink(policy::PowerCapability),
    /// Constract as source
    Source(policy::PowerCapability),
}

/// Port status
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatus {
    /// Current available source contract
    pub available_source_contract: Option<policy::PowerCapability>,
    /// Current available sink contract
    pub available_sink_contract: Option<policy::PowerCapability>,
    /// Current connection state
    pub connection_state: Option<ConnectionState>,
    /// Port partner supports dual-power roles
    pub dual_power: bool,
    /// Active alt-modes
    pub alt_mode: AltMode,
    /// Power path status
    pub power_path: PowerPathStatus,
}

impl PortStatus {
    /// Create a new blank port status
    /// Needed because default() is not const
    pub const fn new() -> Self {
        Self {
            available_source_contract: None,
            available_sink_contract: None,
            connection_state: None,
            dual_power: false,
            alt_mode: AltMode::none(),
            power_path: PowerPathStatus::none(),
        }
    }

    /// Check if the port is connected
    pub fn is_connected(&self) -> bool {
        matches!(
            self.connection_state,
            Some(ConnectionState::Attached)
                | Some(ConnectionState::DebugAccessory)
                | Some(ConnectionState::AudioAccessory)
        )
    }

    /// Check if a debug accessory is connected
    pub fn is_debug_accessory(&self) -> bool {
        matches!(self.connection_state, Some(ConnectionState::DebugAccessory))
    }
}

impl Default for PortStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// Port-specific command data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortCommandData {
    /// Get port status
    PortStatus,
    /// Get and clear events
    ClearEvents,
    /// Get retimer fw update state
    RetimerFwUpdateGetState,
    /// Set retimer fw update state
    RetimerFwUpdateSetState,
    /// Clear retimer fw update state
    RetimerFwUpdateClearState,
    /// Set retimer compliance
    SetRetimerCompliance,
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

/// PD controller command-specific data
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RetimerFwUpdateState {
    /// Retimer FW Update Inactive
    Inactive,
    /// Revimer FW Update Active
    Active,
}

/// Port-specific response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortResponseData {
    /// Command completed with no error
    Complete,
    /// Port status
    PortStatus(PortStatus),
    /// ClearEvents
    ClearEvents(PortEventKind),
    /// Retimer Fw Update status
    RtFwUpdateStatus(RetimerFwUpdateState),
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
    /// Get controller status
    Status,
    /// Sync controller state
    SyncState,
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
pub enum InternalResponseData<'a> {
    /// Command complete
    Complete,
    /// Controller status
    Status(ControllerStatus<'a>),
}

/// Response for controller-specific commands
pub type InternalResponse<'a> = Result<InternalResponseData<'a>, PdError>;

/// PD controller command response
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response<'a> {
    /// Controller response
    Controller(InternalResponse<'a>),
    /// UCSI response passthrough
    Lpm(lpm::Response),
    /// Port response
    Port(PortResponse),
}

/// Controller status
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerStatus<'a> {
    /// Current controller mode
    pub mode: &'a str,
    /// True if we did not have to boot from a backup FW bank
    pub valid_fw_bank: bool,
    /// FW version 0
    pub fw_version0: u32,
    /// FW version 1
    pub fw_version1: u32,
}

/// PD controller
pub struct Device<'a> {
    node: intrusive_list::Node,
    id: ControllerId,
    ports: &'a [GlobalPortId],
    num_ports: usize,
    command: deferred::Channel<NoopRawMutex, Command, Response<'static>>,
}

impl intrusive_list::NodeContainer for Device<'static> {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

impl<'a> Device<'a> {
    /// Create a new PD controller struct
    pub fn new(id: ControllerId, ports: &'a [GlobalPortId]) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            ports,
            num_ports: ports.len(),
            command: deferred::Channel::new(),
        }
    }

    /// Get the controller ID
    pub fn id(&self) -> ControllerId {
        self.id
    }

    /// Send a command to this controller
    pub async fn execute_command(&self, command: Command) -> Response {
        self.command.execute(command).await
    }

    /// Check if this controller has the given port
    pub fn has_port(&self, port: GlobalPortId) -> bool {
        self.lookup_local_port(port).is_ok()
    }

    /// Covert a local port ID to a global port ID
    pub fn lookup_global_port(&self, port: LocalPortId) -> Result<GlobalPortId, PdError> {
        if port.0 >= self.num_ports as u8 {
            return Err(PdError::InvalidParams);
        }

        Ok(self.ports[port.0 as usize])
    }

    /// Convert a global port ID to a local port ID
    pub fn lookup_local_port(&self, port: GlobalPortId) -> Result<LocalPortId, PdError> {
        self.ports
            .iter()
            .position(|p| *p == port)
            .map(|p| LocalPortId(p as u8))
            .ok_or(PdError::InvalidParams)
    }

    /// Create a command handler for this controller
    pub async fn receive(&self) -> deferred::Request<NoopRawMutex, Command, Response<'static>> {
        self.command.receive().await
    }

    /// Notify that there are pending events on one or more ports
    pub async fn notify_ports(&self, pending: PortEventFlags) {
        CONTEXT.get().await.notify_ports(pending);
    }

    /// Number of ports on this controller
    pub fn num_ports(&self) -> usize {
        self.num_ports
    }
}

/// Trait for types that contain a controller struct
pub trait DeviceContainer {
    /// Get the controller struct
    fn get_pd_controller_device(&self) -> &Device<'_>;
}

impl DeviceContainer for Device<'_> {
    fn get_pd_controller_device(&self) -> &Device<'_> {
        self
    }
}

/// PD controller trait that device drivers may use to integrate with internal messaging system
pub trait Controller {
    /// Type of error returned by the bus
    type BusError;

    /// Ensure software state is in sync with hardware state
    fn sync_state(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
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
    /// Returns the retimer fw update state
    fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<RetimerFwUpdateState, Error<Self::BusError>>>;
    /// Set retimer fw update state
    fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Clear retimer fw update state
    fn clear_rt_fw_update_state(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Set retimer compliance
    fn set_rt_compliance(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Enable or disable sink path
    fn enable_sink_path(
        &mut self,
        port: LocalPortId,
        enable: bool,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Get current controller status
    fn get_controller_status(
        &mut self,
    ) -> impl Future<Output = Result<ControllerStatus<'static>, Error<Self::BusError>>>;

    // TODO: remove all these once we migrate to a generic FW update trait
    // https://github.com/OpenDevicePartnership/embedded-services/issues/242
    /// Get current FW version
    fn get_active_fw_version(&self) -> impl Future<Output = Result<u32, Error<Self::BusError>>>;
    /// Start a firmware update
    fn start_fw_update(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Abort a firmware update
    fn abort_fw_update(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Finalize a firmware update
    fn finalize_fw_update(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Write firmware update contents
    fn write_fw_contents(
        &mut self,
        offset: usize,
        data: &[u8],
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
}

/// Internal context for managing PD controllers
struct Context {
    controllers: intrusive_list::IntrusiveList,
    port_events: Signal<NoopRawMutex, PortEventFlags>,
    /// Channel for receiving commands to the type-C service
    external_command: deferred::Channel<NoopRawMutex, external::Command, external::Response<'static>>,
}

impl Context {
    fn new() -> Self {
        Self {
            controllers: intrusive_list::IntrusiveList::new(),
            port_events: Signal::new(),
            external_command: deferred::Channel::new(),
        }
    }

    /// Notify that there are pending events on one or more ports
    /// Each bit corresponds to a global port ID
    fn notify_ports(&self, pending: PortEventFlags) {
        let raw_pending: u32 = pending.into();
        trace!("Notify ports: {:#x}", raw_pending);
        // Early exit if no events
        if pending.is_none() {
            return;
        }

        self.port_events
            .signal(if let Some(flags) = self.port_events.try_take() {
                flags.union(pending)
            } else {
                pending
            });
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

pub(super) async fn lookup_controller(controller_id: ControllerId) -> Result<&'static Device<'static>, PdError> {
    CONTEXT
        .get()
        .await
        .controllers
        .into_iter()
        .filter_map(|node| node.data::<Device>())
        .find(|controller| controller.id == controller_id)
        .ok_or(PdError::InvalidController)
}

/// Default command timeout
/// set to high value since this is intended to prevent an unresponsive device from blocking the service implementation
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// Type to provide access to the PD controller context for service implementations
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
    ) -> Result<InternalResponseData<'static>, PdError> {
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
            .ok_or(PdError::InvalidController)?;

        match node
            .data::<Device>()
            .ok_or(PdError::InvalidController)?
            .execute_command(Command::Controller(command))
            .await
        {
            Response::Controller(response) => response,
            r => {
                error!("Invalid response: expected controller, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Send a command to the given controller with a timeout
    pub async fn send_controller_command(
        &self,
        controller_id: ControllerId,
        command: InternalCommandData,
    ) -> Result<InternalResponseData<'static>, PdError> {
        match with_timeout(
            DEFAULT_TIMEOUT,
            self.send_controller_command_no_timeout(controller_id, command),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => Err(PdError::Timeout),
        }
    }

    /// Reset the given controller
    pub async fn reset_controller(&self, controller_id: ControllerId) -> Result<(), PdError> {
        self.send_controller_command(controller_id, InternalCommandData::Reset)
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
            .execute_command(Command::Lpm(lpm::Command {
                port: port_id,
                operation: command,
            }))
            .await
        {
            Response::Lpm(response) => response,
            r => {
                error!("Invalid response: expected LPM, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Send a command to the given port with a timeout
    pub async fn send_port_command_ucsi(
        &self,
        port_id: GlobalPortId,
        command: lpm::CommandData,
    ) -> Result<lpm::ResponseData, PdError> {
        match with_timeout(
            DEFAULT_TIMEOUT,
            self.send_port_command_ucsi_no_timeout(port_id, command),
        )
        .await
        {
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
        self.send_port_command_ucsi(port_id, lpm::CommandData::ConnectorReset(reset_type))
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
            .execute_command(Command::Port(PortCommand {
                port: port_id,
                data: command,
            }))
            .await
        {
            Response::Port(response) => response,
            r => {
                error!("Invalid response: expected port, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Send a command to the given port with a timeout
    pub async fn send_port_command(
        &self,
        port_id: GlobalPortId,
        command: PortCommandData,
    ) -> Result<PortResponseData, PdError> {
        match with_timeout(DEFAULT_TIMEOUT, self.send_port_command_no_timeout(port_id, command)).await {
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
        match self.send_port_command(port, PortCommandData::ClearEvents).await? {
            PortResponseData::ClearEvents(event) => Ok(event),
            r => {
                error!("Invalid response: expected clear events, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Get the current port status
    pub async fn get_port_status(&self, port: GlobalPortId) -> Result<PortStatus, PdError> {
        match self.send_port_command(port, PortCommandData::PortStatus).await? {
            PortResponseData::PortStatus(status) => Ok(status),
            r => {
                error!("Invalid response: expected port status, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Get the retimer fw update status
    pub async fn get_rt_fw_update_status(&self, port: GlobalPortId) -> Result<RetimerFwUpdateState, PdError> {
        match self
            .send_port_command(port, PortCommandData::RetimerFwUpdateGetState)
            .await?
        {
            PortResponseData::RtFwUpdateStatus(status) => Ok(status),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set the retimer fw update state
    pub async fn set_rt_fw_update_state(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::RetimerFwUpdateSetState)
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Clear the retimer fw update state
    pub async fn clear_rt_fw_update_state(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::RetimerFwUpdateClearState)
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set the retimer compliance
    pub async fn set_rt_compliance(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetRetimerCompliance)
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Get current controller status
    pub async fn get_controller_status(
        &self,
        controller_id: ControllerId,
    ) -> Result<ControllerStatus<'static>, PdError> {
        match self
            .send_controller_command(controller_id, InternalCommandData::Status)
            .await?
        {
            InternalResponseData::Status(status) => Ok(status),
            r => {
                error!("Invalid response: expected controller status, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Sync controller state
    pub async fn sync_controller_state(&self, controller_id: ControllerId) -> Result<(), PdError> {
        match self
            .send_controller_command(controller_id, InternalCommandData::SyncState)
            .await?
        {
            InternalResponseData::Complete => Ok(()),
            r => {
                error!("Invalid response: expected controller status, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Wait for an external command
    pub async fn wait_external_command(
        &self,
    ) -> deferred::Request<NoopRawMutex, external::Command, external::Response<'static>> {
        CONTEXT.get().await.external_command.receive().await
    }

    /// Notify that there are pending events on one or more ports
    pub async fn notify_ports(&self, pending: PortEventFlags) {
        CONTEXT.get().await.notify_ports(pending);
    }
}

/// Execute an external port command
pub(super) async fn execute_external_port_command(
    command: external::Command,
) -> Result<external::PortResponseData, PdError> {
    let context = CONTEXT.get().await;
    match context.external_command.execute(command).await {
        external::Response::Port(response) => response,
        r => {
            error!("Invalid response: expected external port, got {:?}", r);
            Err(PdError::InvalidResponse)
        }
    }
}

/// Execute an external controller command
pub(super) async fn execute_external_controller_command(
    command: external::Command,
) -> Result<external::ControllerResponseData<'static>, PdError> {
    let context = CONTEXT.get().await;
    match context.external_command.execute(command).await {
        external::Response::Controller(response) => response,
        r => {
            error!("Invalid response: expected external controller, got {:?}", r);
            Err(PdError::InvalidResponse)
        }
    }
}
