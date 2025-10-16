//! PD controller related code
use core::future::Future;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, with_timeout};
use embedded_usb_pd::ucsi::{self, lpm};
use embedded_usb_pd::{
    DataRole, Error, GlobalPortId, LocalPortId, PdError, PlugOrientation, PowerRole,
    ado::Ado,
    pdinfo::{AltMode, PowerPathStatus},
    type_c::ConnectionState,
};

use super::{ATTN_VDM_LEN, ControllerId, OTHER_VDM_LEN, external};
use crate::ipc::deferred;
use crate::power::policy;
use crate::type_c::Cached;
use crate::type_c::comms::CommsMessage;
use crate::type_c::event::{PortEvent, PortPending};
use crate::{GlobalRawMutex, IntrusiveNode, broadcaster::immediate as broadcaster, error, intrusive_list, trace};

/// maximum number of data objects in a VDM
pub const MAX_NUM_DATA_OBJECTS: usize = 7; // 7 VDOs of 4 bytes each

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
    /// plug orientation
    pub plug_orientation: PlugOrientation,
    /// power role
    pub power_role: PowerRole,
    /// data role
    pub data_role: DataRole,
    /// Active alt-modes
    pub alt_mode: AltMode,
    /// Power path status
    pub power_path: PowerPathStatus,
    /// EPR mode active
    pub epr: bool,
    /// Port partner is unconstrained
    pub unconstrained_power: bool,
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
            plug_orientation: PlugOrientation::CC1,
            power_role: PowerRole::Sink,
            data_role: DataRole::Dfp,
            alt_mode: AltMode::none(),
            power_path: PowerPathStatus::none(),
            epr: false,
            unconstrained_power: false,
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

/// Other Vdm data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OtherVdm {
    /// Other VDM data
    pub data: [u8; OTHER_VDM_LEN],
}

impl Default for OtherVdm {
    fn default() -> Self {
        Self {
            data: [0; OTHER_VDM_LEN],
        }
    }
}

impl From<OtherVdm> for [u8; OTHER_VDM_LEN] {
    fn from(vdm: OtherVdm) -> Self {
        vdm.data
    }
}

impl From<[u8; OTHER_VDM_LEN]> for OtherVdm {
    fn from(data: [u8; OTHER_VDM_LEN]) -> Self {
        Self { data }
    }
}

/// Attention Vdm data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AttnVdm {
    /// Attention VDM data
    pub data: [u8; ATTN_VDM_LEN],
}

/// DisplayPort pin configuration
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpPinConfig {
    /// 4L DP connection using USBC-USBC cable (Pin Assignment C)
    pub pin_c: bool,
    /// 2L USB + 2L DP connection using USBC-USBC cable (Pin Assignment D)
    pub pin_d: bool,
    /// 4L DP connection using USBC-DP cable (Pin Assignment E)
    pub pin_e: bool,
}

/// DisplayPort status data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpStatus {
    /// DP alt-mode entered
    pub alt_mode_entered: bool,
    /// Get DP DFP pin config
    pub dfp_d_pin_cfg: DpPinConfig,
}

/// DisplayPort configuration data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpConfig {
    /// DP alt-mode enabled
    pub enable: bool,
    /// Set DP DFP pin config
    pub dfp_d_pin_cfg: DpPinConfig,
}

impl Default for AttnVdm {
    fn default() -> Self {
        Self {
            data: [0; ATTN_VDM_LEN],
        }
    }
}

impl From<AttnVdm> for [u8; ATTN_VDM_LEN] {
    fn from(vdm: AttnVdm) -> Self {
        vdm.data
    }
}

impl From<[u8; ATTN_VDM_LEN]> for AttnVdm {
    fn from(data: [u8; ATTN_VDM_LEN]) -> Self {
        Self { data }
    }
}

/// Send VDM data
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SendVdm {
    /// initiating a VDM sequence
    pub initiator: bool,
    /// VDO count
    pub vdo_count: u8,
    /// VDO data
    pub vdo_data: [u32; MAX_NUM_DATA_OBJECTS],
}

impl SendVdm {
    /// Create a new blank port status
    pub const fn new() -> Self {
        Self {
            initiator: false,
            vdo_count: 0,
            vdo_data: [0; MAX_NUM_DATA_OBJECTS],
        }
    }
}

impl Default for SendVdm {
    fn default() -> Self {
        Self::new()
    }
}

/// USB control configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbControlConfig {
    /// Enable USB2 data path
    pub usb2_enabled: bool,
    /// Enable USB3 data path  
    pub usb3_enabled: bool,
    /// Enable USB4 data path
    pub usb4_enabled: bool,
}

impl Default for UsbControlConfig {
    fn default() -> Self {
        Self {
            usb2_enabled: true,
            usb3_enabled: true,
            usb4_enabled: true,
        }
    }
}

/// Thunderbolt control configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Default, Copy, PartialEq)]
pub struct TbtConfig {
    /// Enable Thunderbolt
    pub tbt_enabled: bool,
}

/// PD state-machine configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Default, Copy, PartialEq)]
pub struct PdStateMachineConfig {
    /// Enable or disable the PD state-machine
    pub enabled: bool,
}

/// TypeC State Machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TypeCStateMachineState {
    /// Sink state machine only
    Sink,
    /// Source state machine only
    Source,
    /// DRP state machine
    Drp,
    /// Disabled
    Disabled,
}

/// Port-specific command data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortCommandData {
    /// Get port status
    PortStatus(Cached),
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
    /// Reconfigure retimer
    ReconfigureRetimer,
    /// Get oldest unhandled PD alert
    GetPdAlert,
    /// Set the maximum sink voltage in mV for the given port
    SetMaxSinkVoltage(Option<u16>),
    /// Set unconstrained power
    SetUnconstrainedPower(bool),
    /// Clear the dead battery flag for the given port
    ClearDeadBatteryFlag,
    /// Get other VDM
    GetOtherVdm,
    /// Get attention VDM
    GetAttnVdm,
    /// Send VDM
    SendVdm(SendVdm),
    /// Set USB control configuration
    SetUsbControl(UsbControlConfig),
    /// Get DisplayPort status
    GetDpStatus,
    /// Set DisplayPort configuration
    SetDpConfig(DpConfig),
    /// Execute DisplayPort reset
    ExecuteDrst,
    /// Set Thunderbolt configuration
    SetTbtConfig(TbtConfig),
    /// Set PD state-machine configuration
    SetPdStateMachineConfig(PdStateMachineConfig),
    /// Set Type-C state-machine configuration
    SetTypeCStateMachineConfig(TypeCStateMachineState),
    /// Execute the UCSI command
    ExecuteUcsiCommand(lpm::CommandData),
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
    ClearEvents(PortEvent),
    /// Retimer Fw Update status
    RtFwUpdateStatus(RetimerFwUpdateState),
    /// PD alert
    PdAlert(Option<Ado>),
    /// Get other VDM
    OtherVdm(OtherVdm),
    /// Get attention VDM
    AttnVdm(AttnVdm),
    /// Get DisplayPort status
    DpStatus(DpStatus),
    /// UCSI response
    UcsiResponse(Result<Option<lpm::ResponseData>, PdError>),
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
    Lpm(lpm::GlobalCommand),
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
    Ucsi(ucsi::GlobalResponse),
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
    command: deferred::Channel<GlobalRawMutex, Command, Response<'static>>,
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
    pub async fn execute_command(&self, command: Command) -> Response<'_> {
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
    ///
    /// DROP SAFETY: Direct call to deferred channel primitive
    pub async fn receive(&self) -> deferred::Request<'_, GlobalRawMutex, Command, Response<'static>> {
        self.command.receive().await
    }

    /// Notify that there are pending events on one or more ports
    pub async fn notify_ports(&self, pending: PortPending) {
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

    /// Wait for a port event to occur
    /// # Implementation guide
    /// This function should be drop safe.
    /// Any intermediate side effects must be undone if the returned [`Future`] is dropped before completing.
    fn wait_port_event(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Returns and clears current events for the given port
    /// # Implementation guide
    /// This function should be drop safe.
    /// Any intermediate side effects must be undone if the returned [`Future`] is dropped before completing.
    fn clear_port_events(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<PortEvent, Error<Self::BusError>>>;
    /// Returns the port status
    fn get_port_status(&mut self, port: LocalPortId)
    -> impl Future<Output = Result<PortStatus, Error<Self::BusError>>>;

    /// Reset the controller
    fn reset_controller(&mut self) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

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

    /// Reconfigure the retimer for the given port.
    fn reconfigure_retimer(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Clear the dead battery flag for the given port.
    fn clear_dead_battery_flag(&mut self, port: LocalPortId)
    -> impl Future<Output = Result<(), Error<Self::BusError>>>;

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
    /// Get current PD alert
    fn get_pd_alert(&mut self, port: LocalPortId) -> impl Future<Output = Result<Option<Ado>, Error<Self::BusError>>>;
    /// Set the maximum sink voltage for the given port
    ///
    /// This may trigger a renegotiation
    fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Set port unconstrained status
    fn set_unconstrained_power(
        &mut self,
        port: LocalPortId,
        unconstrained: bool,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    // TODO: remove all these once we migrate to a generic FW update trait
    // https://github.com/OpenDevicePartnership/embedded-services/issues/242
    /// Get current FW version
    fn get_active_fw_version(&mut self) -> impl Future<Output = Result<u32, Error<Self::BusError>>>;
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
    /// Get the Rx Other VDM data for the given port
    fn get_other_vdm(&mut self, port: LocalPortId) -> impl Future<Output = Result<OtherVdm, Error<Self::BusError>>>;
    /// Get the Rx Attention VDM data for the given port
    fn get_attn_vdm(&mut self, port: LocalPortId) -> impl Future<Output = Result<AttnVdm, Error<Self::BusError>>>;
    /// Send a VDM to the given port
    fn send_vdm(
        &mut self,
        port: LocalPortId,
        tx_vdm: SendVdm,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Set USB control configuration for the given port
    fn set_usb_control(
        &mut self,
        port: LocalPortId,
        config: UsbControlConfig,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Get DisplayPort status for the given port
    fn get_dp_status(&mut self, port: LocalPortId) -> impl Future<Output = Result<DpStatus, Error<Self::BusError>>>;
    /// Set DisplayPort configuration for the given port
    fn set_dp_config(
        &mut self,
        port: LocalPortId,
        config: DpConfig,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;
    /// Execute PD Data Reset for the given port
    fn execute_drst(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Set Thunderbolt configuration for the given port
    fn set_tbt_config(
        &mut self,
        port: LocalPortId,
        config: TbtConfig,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Set PD state-machine configuration for the given port
    fn set_pd_state_machine_config(
        &mut self,
        port: LocalPortId,
        config: PdStateMachineConfig,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Set Type-C state-machine configuration for the given port
    fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: TypeCStateMachineState,
    ) -> impl Future<Output = Result<(), Error<Self::BusError>>>;

    /// Execute the given UCSI command
    fn execute_ucsi_command(
        &mut self,
        command: lpm::LocalCommand,
    ) -> impl Future<Output = Result<Option<lpm::ResponseData>, Error<Self::BusError>>>;
}

/// Internal context for managing PD controllers
struct Context {
    controllers: intrusive_list::IntrusiveList,
    port_events: Signal<GlobalRawMutex, PortPending>,
    /// Channel for receiving commands to the type-C service
    external_command: deferred::Channel<GlobalRawMutex, external::Command, external::Response<'static>>,
    /// Event broadcaster
    broadcaster: broadcaster::Immediate<CommsMessage>,
}

impl Context {
    fn new() -> Self {
        Self {
            controllers: intrusive_list::IntrusiveList::new(),
            port_events: Signal::new(),
            external_command: deferred::Channel::new(),
            broadcaster: broadcaster::Immediate::default(),
        }
    }

    /// Notify that there are pending events on one or more ports
    /// Each bit corresponds to a global port ID
    fn notify_ports(&self, pending: PortPending) {
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

/// Get total number of ports on the system
pub(super) async fn get_num_ports() -> usize {
    CONTEXT
        .get()
        .await
        .controllers
        .iter_only::<Device>()
        .fold(0, |acc, controller| acc + controller.num_ports())
}

/// Register a message receiver for type-C messages
pub async fn register_message_receiver(
    receiver: &'static broadcaster::Receiver<'_, CommsMessage>,
) -> intrusive_list::Result<()> {
    CONTEXT.get().await.broadcaster.register_receiver(receiver)
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
    ) -> Result<ucsi::GlobalResponse, PdError> {
        let node = self.find_node_by_port(port_id).await?;

        match node
            .data::<Device>()
            .ok_or(PdError::InvalidController)?
            .execute_command(Command::Lpm(lpm::Command::new(port_id, command)))
            .await
        {
            Response::Ucsi(response) => Ok(response),
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
    ) -> Result<ucsi::GlobalResponse, PdError> {
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
    pub async fn get_unhandled_events(&self) -> PortPending {
        CONTEXT.get().await.port_events.wait().await
    }

    /// Get the unhandled events for the given port
    pub async fn get_port_event(&self, port: GlobalPortId) -> Result<PortEvent, PdError> {
        match self.send_port_command(port, PortCommandData::ClearEvents).await? {
            PortResponseData::ClearEvents(event) => Ok(event),
            r => {
                error!("Invalid response: expected clear events, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Get the current port status
    pub async fn get_port_status(&self, port: GlobalPortId, cached: Cached) -> Result<PortStatus, PdError> {
        match self
            .send_port_command(port, PortCommandData::PortStatus(cached))
            .await?
        {
            PortResponseData::PortStatus(status) => Ok(status),
            r => {
                error!("Invalid response: expected port status, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Get the oldest unhandled PD alert for the given port
    pub async fn get_pd_alert(&self, port: GlobalPortId) -> Result<Option<Ado>, PdError> {
        match self.send_port_command(port, PortCommandData::GetPdAlert).await? {
            PortResponseData::PdAlert(alert) => Ok(alert),
            r => {
                error!("Invalid response: expected PD alert, got {:?}", r);
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

    /// Reconfigure the retimer for the given port.
    pub async fn reconfigure_retimer(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::ReconfigureRetimer)
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set the maximum sink voltage for the given port.
    ///
    /// See [`PortCommandData::SetMaxSinkVoltage`] for details on the `max_voltage_mv` parameter.
    pub async fn set_max_sink_voltage(&self, port: GlobalPortId, max_voltage_mv: Option<u16>) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetMaxSinkVoltage(max_voltage_mv))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Clear the dead battery flag for the given port.
    pub async fn clear_dead_battery_flag(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::ClearDeadBatteryFlag)
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

    /// Set unconstrained power for the given port
    pub async fn set_unconstrained_power(&self, port: GlobalPortId, unconstrained: bool) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetUnconstrainedPower(unconstrained))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
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
    ) -> deferred::Request<'_, GlobalRawMutex, external::Command, external::Response<'static>> {
        CONTEXT.get().await.external_command.receive().await
    }

    /// Notify that there are pending events on one or more ports
    pub async fn notify_ports(&self, pending: PortPending) {
        CONTEXT.get().await.notify_ports(pending);
    }

    /// Get the number of ports on the system
    pub async fn get_num_ports(&self) -> usize {
        get_num_ports().await
    }

    /// Get the other vdm for the given port
    pub async fn get_other_vdm(&self, port: GlobalPortId) -> Result<OtherVdm, PdError> {
        match self.send_port_command(port, PortCommandData::GetOtherVdm).await? {
            PortResponseData::OtherVdm(vdm) => Ok(vdm),
            r => {
                error!("Invalid response: expected other VDM, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Get the attention vdm for the given port
    pub async fn get_attn_vdm(&self, port: GlobalPortId) -> Result<AttnVdm, PdError> {
        match self.send_port_command(port, PortCommandData::GetAttnVdm).await? {
            PortResponseData::AttnVdm(vdm) => Ok(vdm),
            r => {
                error!("Invalid response: expected attention VDM, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Send VDM to the given port
    pub async fn send_vdm(&self, port: GlobalPortId, tx_vdm: SendVdm) -> Result<(), PdError> {
        match self.send_port_command(port, PortCommandData::SendVdm(tx_vdm)).await? {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set USB control configuration for the given port
    pub async fn set_usb_control(&self, port: GlobalPortId, config: UsbControlConfig) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetUsbControl(config))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Get DisplayPort status for the given port
    pub async fn get_dp_status(&self, port: GlobalPortId) -> Result<DpStatus, PdError> {
        match self.send_port_command(port, PortCommandData::GetDpStatus).await? {
            PortResponseData::DpStatus(status) => Ok(status),
            r => {
                error!("Invalid response: expected DP status, got {:?}", r);
                Err(PdError::InvalidResponse)
            }
        }
    }

    /// Set DisplayPort configuration for the given port
    pub async fn set_dp_config(&self, port: GlobalPortId, config: DpConfig) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetDpConfig(config))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Execute PD Data Reset for the given port
    pub async fn execute_drst(&self, port: GlobalPortId) -> Result<(), PdError> {
        match self.send_port_command(port, PortCommandData::ExecuteDrst).await? {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set Thunderbolt configuration for the given port
    pub async fn set_tbt_config(&self, port: GlobalPortId, config: TbtConfig) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetTbtConfig(config))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set PD state-machine configuration for the given port
    pub async fn set_pd_state_machine_config(
        &self,
        port: GlobalPortId,
        config: PdStateMachineConfig,
    ) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetPdStateMachineConfig(config))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Set Type-C state-machine configuration for the given port
    pub async fn set_type_c_state_machine_config(
        &self,
        port: GlobalPortId,
        state: TypeCStateMachineState,
    ) -> Result<(), PdError> {
        match self
            .send_port_command(port, PortCommandData::SetTypeCStateMachineConfig(state))
            .await?
        {
            PortResponseData::Complete => Ok(()),
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Execute the given UCSI command
    pub async fn execute_ucsi_command(
        &self,
        command: lpm::GlobalCommand,
    ) -> Result<Option<lpm::ResponseData>, PdError> {
        match self
            .send_port_command(command.port(), PortCommandData::ExecuteUcsiCommand(command.operation()))
            .await?
        {
            PortResponseData::UcsiResponse(response) => response,
            _ => Err(PdError::InvalidResponse),
        }
    }

    /// Broadcast a type-C message to all subscribers
    pub async fn broadcast_message(&self, message: CommsMessage) {
        CONTEXT.get().await.broadcaster.broadcast(message).await;
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

/// Execute an external UCSI command
pub(super) async fn execute_external_ucsi_command(command: ucsi::GlobalCommand) -> external::UcsiResponse {
    let context = CONTEXT.get().await;
    match context.external_command.execute(external::Command::Ucsi(command)).await {
        external::Response::Ucsi(response) => response,
        r => {
            error!("Invalid response: expected external UCSI, got {:?}", r);
            external::UcsiResponse {
                // Always notify OPM of an error
                notify_opm: true,
                cci: ucsi::cci::GlobalCci::new_error(),
                data: Err(PdError::InvalidResponse),
            }
        }
    }
}
