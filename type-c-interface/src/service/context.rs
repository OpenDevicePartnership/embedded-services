use embassy_sync::signal::Signal;
use embassy_time::{Duration, with_timeout};
use embedded_usb_pd::ucsi::{self, lpm};
use embedded_usb_pd::{GlobalPortId, PdError, ado::Ado};

use crate::port::event::{PortEvent, PortPending};
use crate::port::{
    AttnVdm, Command, ControllerStatus, Device, DpConfig, DpStatus, InternalCommandData, InternalResponseData,
    OtherVdm, PdStateMachineConfig, PortCommand, PortCommandData, PortResponseData, PortStatus, Response,
    RetimerFwUpdateState, SendVdm, TbtConfig, TypeCStateMachineState, UsbControlConfig,
};
use crate::port::{Cached, ControllerId};
use crate::service;
use crate::service::event::Event;
use embedded_services::{
    GlobalRawMutex, IntrusiveNode, broadcaster::immediate as broadcaster, error, intrusive_list, trace,
};

/// Default command timeout
/// set to high value since this is intended to prevent an unresponsive device from blocking the service implementation
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

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

pub struct Context {
    port_events: Signal<GlobalRawMutex, PortPending>,
    /// Event broadcaster
    broadcaster: broadcaster::Immediate<Event>,
    /// Controller list
    controllers: intrusive_list::IntrusiveList,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Create new Context
    pub const fn new() -> Self {
        Self {
            port_events: Signal::new(),
            broadcaster: broadcaster::Immediate::new(),
            controllers: intrusive_list::IntrusiveList::new(),
        }
    }

    /// Notify that there are pending events on one or more ports
    /// Each bit corresponds to a global port ID
    pub fn notify_ports(&self, pending: PortPending) {
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

    /// Send a command to the given controller with no timeout
    pub async fn send_controller_command_no_timeout(
        &self,
        controller_id: ControllerId,
        command: InternalCommandData,
    ) -> Result<InternalResponseData<'static>, PdError> {
        let node = self
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

    fn find_node_by_port(&self, port_id: GlobalPortId) -> Result<&IntrusiveNode, PdError> {
        self.controllers
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
        let node = self.find_node_by_port(port_id)?;

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
        let node = self.find_node_by_port(port_id)?;

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
        self.port_events.wait().await
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

    /// Register a message receiver for type-C messages
    pub async fn register_message_receiver(
        &self,
        receiver: &'static broadcaster::Receiver<'_, service::event::Event>,
    ) -> intrusive_list::Result<()> {
        self.broadcaster.register_receiver(receiver)
    }

    /// Broadcast a type-C message to all subscribers
    pub async fn broadcast_message(&self, message: service::event::Event) {
        self.broadcaster.broadcast(message).await;
    }

    /// Register a PD controller
    pub fn register_controller(&self, controller: &'static impl DeviceContainer) -> Result<(), intrusive_list::Error> {
        self.controllers.push(controller.get_pd_controller_device())
    }

    /// Get total number of ports on the system
    pub fn get_num_ports(&self) -> usize {
        self.controllers
            .iter_only::<Device>()
            .fold(0, |acc, controller| acc + controller.num_ports())
    }
}
