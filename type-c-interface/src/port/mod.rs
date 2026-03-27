//! PD controller related code
use core::future::Future;

use embedded_usb_pd::ucsi::{self, lpm};
use embedded_usb_pd::{
    DataRole, Error, GlobalPortId, LocalPortId, PdError, PlugOrientation, PowerRole,
    ado::Ado,
    pdinfo::{AltMode, PowerPathStatus},
    type_c::ConnectionState,
};

use embedded_services::ipc::deferred;
use embedded_services::{GlobalRawMutex, intrusive_list};

pub mod event;

use event::{PortEvent, PortPending};

/// Length of the Other VDM data
pub const OTHER_VDM_LEN: usize = 29;
/// Length of the Attention VDM data
pub const ATTN_VDM_LEN: usize = 9;
/// maximum number of data objects in a VDM
pub const MAX_NUM_DATA_OBJECTS: usize = 7; // 7 VDOs of 4 bytes each

/// Newtype to help clarify arguments to port status commands
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cached(pub bool);

/// Controller ID
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerId(pub u8);

/// Port status
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatus {
    /// Current available source contract
    pub available_source_contract: Option<power_policy_interface::capability::PowerCapability>,
    /// Current available sink contract
    pub available_sink_contract: Option<power_policy_interface::capability::PowerCapability>,
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
    /// Retimer FW Update Active
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

    /// Convert a local port ID to a global port ID
    pub fn lookup_global_port(&self, port: LocalPortId) -> Result<GlobalPortId, PdError> {
        Ok(*self.ports.get(port.0 as usize).ok_or(PdError::InvalidParams)?)
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
    pub fn notify_ports(&self, ctx: &crate::service::context::Context, pending: PortPending) {
        ctx.notify_ports(pending);
    }

    /// Number of ports on this controller
    pub fn num_ports(&self) -> usize {
        self.num_ports
    }

    /// Slice of global ports on the Device
    pub fn ports(&self) -> &'a [GlobalPortId] {
        self.ports
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
