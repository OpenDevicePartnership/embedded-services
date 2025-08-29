use embedded_services::{
    debug, error,
    type_c::{controller::DpConfig, external},
};
use embedded_usb_pd::GlobalPortId;

use super::*;
use crate::PortEventStreamer;

use embedded_services::type_c::controller::SendVdm;

impl<'a> Service<'a> {
    /// Wait for port flags
    pub(super) async fn wait_port_flags(&self) -> PortEventStreamer {
        if let Some(ref streamer) = self.state.lock().await.port_event_streaming_state {
            // If we have an existing iterator, return it
            // Yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            *streamer
        } else {
            // Wait for the next port event and create a streamer
            PortEventStreamer::new(self.context.get_unhandled_events().await.into_iter())
        }
    }

    /// Process external port commands
    pub(super) async fn process_external_port_command(
        &self,
        command: &external::PortCommand,
    ) -> external::Response<'static> {
        debug!("Processing external port command: {:#?}", command);
        match command.data {
            external::PortCommandData::PortStatus(cached) => {
                self.process_external_port_status(command.port, cached).await
            }
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
            external::PortCommandData::ReconfigureRetimer => self.process_reconfigure_retimer(command.port).await,
            external::PortCommandData::SetMaxSinkVoltage { max_voltage_mv } => {
                self.process_set_max_sink_voltage(command.port, max_voltage_mv).await
            }
            external::PortCommandData::ClearDeadBatteryFlag => self.process_clear_dead_battery_flag(command.port).await,
            external::PortCommandData::SendVdm(tx_vdm) => self.process_send_vdm(command.port, tx_vdm).await,
            external::PortCommandData::GetDpStatus => self.process_get_dp_status(command.port).await,
            external::PortCommandData::SetDpConfig(config) => self.process_set_dp_config(command.port, config).await,
            external::PortCommandData::ExecuteDrst => self.process_execute_drst(command.port).await,
        }
    }

    /// Process external port status command
    pub(super) async fn process_external_port_status(
        &self,
        port_id: GlobalPortId,
        cached: Cached,
    ) -> external::Response<'static> {
        let status = self.context.get_port_status(port_id, cached).await;
        if let Err(e) = status {
            error!("Error getting port status: {:#?}", e);
        }
        external::Response::Port(status.map(external::PortResponseData::PortStatus))
    }

    /// Process get retimer fw update status commands
    pub(super) async fn process_get_rt_fw_update_status(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.get_rt_fw_update_status(port_id).await;
        if let Err(e) = status {
            error!("Error getting retimer fw update status: {:#?}", e);
        }

        external::Response::Port(status.map(external::PortResponseData::RetimerFwUpdateGetState))
    }

    /// Process set retimer fw update state commands
    pub(super) async fn process_set_rt_fw_update_state(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.set_rt_fw_update_state(port_id).await;
        if let Err(e) = status {
            error!("Error setting retimer fw update state: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process clear retimer fw update state commands
    pub(super) async fn process_clear_rt_fw_update_state(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.clear_rt_fw_update_state(port_id).await;
        if let Err(e) = status {
            error!("Error clear retimer fw update state: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process set retimer compliance
    pub(super) async fn process_set_rt_compliance(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.set_rt_compliance(port_id).await;
        if let Err(e) = status {
            error!("Error set retimer compliance: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_reconfigure_retimer(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.reconfigure_retimer(port_id).await;
        if let Err(e) = status {
            error!("Error reconfiguring retimer: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_set_max_sink_voltage(
        &self,
        port_id: GlobalPortId,
        max_voltage_mv: Option<u16>,
    ) -> external::Response<'static> {
        let status = self.context.set_max_sink_voltage(port_id, max_voltage_mv).await;
        if let Err(e) = status {
            error!("Error setting max voltage: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    async fn process_clear_dead_battery_flag(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.clear_dead_battery_flag(port_id).await;
        if let Err(e) = status {
            error!("Error clearing dead battery flag: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process send vdm commands
    async fn process_send_vdm(&self, port_id: GlobalPortId, tx_vdm: SendVdm) -> external::Response<'static> {
        let status = self.context.send_vdm(port_id, tx_vdm).await;
        if let Err(e) = status {
            error!("Error sending VDM data: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process get DisplayPort status commands
    async fn process_get_dp_status(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.get_dp_status(port_id).await;
        if let Err(e) = status {
            error!("Error getting DP status: {:#?}", e);
        }

        external::Response::Port(status.map(|dp_status| external::PortResponseData::GetDpStatus(dp_status)))
    }

    /// Process set DisplayPort config commands
    async fn process_set_dp_config(&self, port_id: GlobalPortId, config: DpConfig) -> external::Response<'static> {
        let status = self.context.set_dp_config(port_id, config).await;
        if let Err(e) = status {
            error!("Error setting DP config: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }

    /// Process execute DisplayPort reset commands
    async fn process_execute_drst(&self, port_id: GlobalPortId) -> external::Response<'static> {
        let status = self.context.execute_drst(port_id).await;
        if let Err(e) = status {
            error!("Error executing DP reset: {:#?}", e);
        }

        external::Response::Port(status.map(|_| external::PortResponseData::Complete))
    }
}
