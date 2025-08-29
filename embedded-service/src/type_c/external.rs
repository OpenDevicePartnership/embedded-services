//! Message definitions for external type-C commands
use embedded_usb_pd::{GlobalPortId, LocalPortId, PdError, ucsi};

use crate::type_c::{Cached, controller::execute_external_ucsi_command};

use super::{
    ControllerId,
    controller::{
        ControllerStatus, DpConfig, DpStatus, PortStatus, RetimerFwUpdateState, SendVdm,
        execute_external_controller_command, execute_external_port_command, lookup_controller,
    },
};

/// Data for controller-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControllerCommandData {
    /// Get controller status
    ControllerStatus,
    /// Sync controller state
    SyncState,
    /// Controller reset
    Reset,
}

/// Controller-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerCommand {
    /// Controller ID
    pub id: ControllerId,
    /// Command data
    pub data: ControllerCommandData,
}

/// Response data for controller-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControllerResponseData<'a> {
    /// Command complete
    Complete,
    /// Get controller status
    ControllerStatus(ControllerStatus<'a>),
}

/// Controller-specific command response
pub type ControllerResponse<'a> = Result<ControllerResponseData<'a>, PdError>;

/// Data for port-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortCommandData {
    /// Get port status. The `bool` argument indicates whether to use cached data or force a fetch of register values.
    PortStatus(Cached),
    /// Get retimer fw update status
    RetimerFwUpdateGetState,
    /// Set retimer fw update status
    RetimerFwUpdateSetState,
    /// Clear retimer fw update status
    RetimerFwUpdateClearState,
    /// Set retimer compliance
    SetRetimerCompliance,
    /// Reconfigure retimer
    ReconfigureRetimer,
    /// Set max sink voltage to a specific value.
    SetMaxSinkVoltage {
        /// The maximum voltage to set, in millivolts.
        /// If [`None`], the port will be set to its default maximum voltage.
        max_voltage_mv: Option<u16>,
    },
    /// Clear the dead battery flag for the given port.
    ClearDeadBatteryFlag,
    /// Send VDM
    SendVdm(SendVdm),
    /// Get DisplayPort status
    GetDpStatus,
    /// Set DisplayPort configuration
    SetDpConfig(DpConfig),
    /// Execute DisplayPort reset
    ExecuteDrst,
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

/// Response data for port-specific commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortResponseData {
    /// Command completed with no error
    Complete,
    /// Get port status
    PortStatus(PortStatus),
    /// Get retimer fw update status
    RetimerFwUpdateGetState(RetimerFwUpdateState),
    /// Get DisplayPort status
    GetDpStatus(DpStatus),
}

/// Port-specific command response
pub type PortResponse = Result<PortResponseData, PdError>;

/// External commands for type-C service
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    /// Port command
    Port(PortCommand),
    /// Controller command
    Controller(ControllerCommand),
    /// UCSI command
    Ucsi(ucsi::GlobalCommand),
}

/// UCSI command response
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UcsiResponse {
    /// Notify the OPM, the function call
    pub notify_opm: bool,
    /// UCSI response
    pub response: ucsi::GlobalResponse,
}

/// External command response for type-C service
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response<'a> {
    /// Port command response
    Port(PortResponse),
    /// Controller command response
    Controller(ControllerResponse<'a>),
    /// UCSI command response
    Ucsi(Result<UcsiResponse, PdError>),
}

/// Get the status of the given port.
///
/// Use the `cached` argument to specify whether to use cached data or force a fetch of register values.
pub async fn get_port_status(port: GlobalPortId, cached: Cached) -> Result<PortStatus, PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::PortStatus(cached),
    }))
    .await?
    {
        PortResponseData::PortStatus(status) => Ok(status),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Get the status of the given port by its controller and local port ID.
///
/// Use the `cached` argument to specify whether to use cached data or force a fetch of register values.
pub async fn get_controller_port_status(
    controller: ControllerId,
    port: LocalPortId,
    cached: Cached,
) -> Result<PortStatus, PdError> {
    let global_port = controller_port_to_global_id(controller, port).await?;
    get_port_status(global_port, cached).await
}

/// Reset the given controller.
pub async fn reset_controller(controller_id: ControllerId) -> Result<(), PdError> {
    match execute_external_controller_command(Command::Controller(ControllerCommand {
        id: controller_id,
        data: ControllerCommandData::Reset,
    }))
    .await?
    {
        ControllerResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Get the status of the given controller
#[allow(unreachable_patterns)]
pub async fn get_controller_status(id: ControllerId) -> Result<ControllerStatus<'static>, PdError> {
    match execute_external_controller_command(Command::Controller(ControllerCommand {
        id,
        data: ControllerCommandData::ControllerStatus,
    }))
    .await?
    {
        ControllerResponseData::ControllerStatus(status) => Ok(status),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Get the number of ports on the given controller
pub async fn get_controller_num_ports(controller_id: ControllerId) -> Result<usize, PdError> {
    Ok(lookup_controller(controller_id).await?.num_ports())
}

/// Convert a (controller ID, local port ID) to a global port ID
pub async fn controller_port_to_global_id(
    controller_id: ControllerId,
    port_id: LocalPortId,
) -> Result<GlobalPortId, PdError> {
    lookup_controller(controller_id).await?.lookup_global_port(port_id)
}

/// Get the retimer fw update status of the given port
pub async fn port_get_rt_fw_update_status(port: GlobalPortId) -> Result<RetimerFwUpdateState, PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::RetimerFwUpdateGetState,
    }))
    .await?
    {
        PortResponseData::RetimerFwUpdateGetState(status) => Ok(status),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Set the retimer fw update state of the given port
pub async fn port_set_rt_fw_update_state(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::RetimerFwUpdateSetState,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Clear the retimer fw update state of the given port
pub async fn port_clear_rt_fw_update_state(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::RetimerFwUpdateClearState,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Set the retimer comliance state of the given port
pub async fn port_set_rt_compliance(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::SetRetimerCompliance,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Trigger a sync of the controller state
pub async fn sync_controller_state(id: ControllerId) -> Result<(), PdError> {
    match execute_external_controller_command(Command::Controller(ControllerCommand {
        id,
        data: ControllerCommandData::SyncState,
    }))
    .await?
    {
        ControllerResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Get number of ports on the system
pub async fn get_num_ports() -> usize {
    super::controller::get_num_ports().await
}

/// Set the maximum voltage for the given port to a specific value.
///
/// See [`PortCommandData::SetMaxSinkVoltage::max_voltage_mv`] for details on the `max_voltage_mv` parameter.
pub async fn set_max_sink_voltage(port: GlobalPortId, max_voltage_mv: Option<u16>) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::SetMaxSinkVoltage { max_voltage_mv },
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Clear the dead battery flag for the given port.
pub async fn clear_dead_battery_flag(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::ClearDeadBatteryFlag,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Reconfigure the retimer for the given port.
pub async fn reconfigure_retimer(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::ReconfigureRetimer,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Execute a UCSI command
pub async fn execute_ucsi_command(command: ucsi::GlobalCommand) -> Result<UcsiResponse, PdError> {
    execute_external_ucsi_command(command).await
}

/// Send vdm to the given port
pub async fn send_vdm(port: GlobalPortId, tx_vdm: SendVdm) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::SendVdm(tx_vdm),
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Get DisplayPort status for the given port
pub async fn get_dp_status(port: GlobalPortId) -> Result<DpStatus, PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::GetDpStatus,
    }))
    .await?
    {
        PortResponseData::GetDpStatus(status) => Ok(status),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Set DisplayPort configuration for the given port
pub async fn set_dp_config(port: GlobalPortId, config: DpConfig) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::SetDpConfig(config),
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}

/// Execute DisplayPort reset for the given port
pub async fn execute_drst(port: GlobalPortId) -> Result<(), PdError> {
    match execute_external_port_command(Command::Port(PortCommand {
        port,
        data: PortCommandData::ExecuteDrst,
    }))
    .await?
    {
        PortResponseData::Complete => Ok(()),
        _ => Err(PdError::InvalidResponse),
    }
}
