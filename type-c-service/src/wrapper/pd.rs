use embedded_services::type_c::controller::InternalResponseData;
use embedded_usb_pd::ucsi::lpm;

use super::*;

impl<const N: usize, C: Controller> ControllerWrapper<'_, N, C> {
    /// Handle a port command
    async fn process_port_command(&self, controller: &mut C, command: controller::PortCommand) {
        let local_port = self.pd_controller.lookup_local_port(command.port);
        if local_port.is_err() {
            self.pd_controller
                .send_response(controller::Response::Port(Err(PdError::InvalidPort)))
                .await;
            return;
        }

        let local_port = local_port.unwrap();
        let response = match command.data {
            controller::PortCommandData::PortStatus => match controller.get_port_status(local_port).await {
                Ok(status) => Ok(controller::PortResponseData::PortStatus(status)),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::ClearEvents => {
                let event = self.active_events[0].get();
                self.active_events[0].set(PortEventKind::none());
                Ok(controller::PortResponseData::ClearEvents(event))
            }
            controller::PortCommandData::RetimerFwUpdateGetState => {
                match controller.get_rt_fw_update_status(local_port).await {
                    Ok(status) => Ok(controller::PortResponseData::RtFwUpdateStatus(status)),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            controller::PortCommandData::RetimerFwUpdateSetState => {
                match controller.set_rt_fw_update_state(local_port).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            controller::PortCommandData::RetimerFwUpdateClearState => {
                match controller.clear_rt_fw_update_state(local_port).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
        };

        self.pd_controller
            .send_response(controller::Response::Port(response))
            .await;
    }

    async fn process_controller_command(&self, controller: &mut C, command: controller::InternalCommandData) {
        let response = match command {
            controller::InternalCommandData::Status => {
                let status = controller.get_controller_status().await;
                controller::Response::Controller(status.map(InternalResponseData::Status).map_err(|_| PdError::Failed))
            }
            _ => controller::Response::Controller(Err(PdError::UnrecognizedCommand)),
        };

        self.pd_controller.send_response(response).await;
    }

    /// Handle a PD controller command
    pub(super) async fn process_pd_command(&self, controller: &mut C, command: controller::Command) {
        match command {
            controller::Command::Port(command) => {
                self.process_port_command(controller, command).await;
            }
            controller::Command::Controller(command) => {
                self.process_controller_command(controller, command).await;
            }
            controller::Command::Lpm(_) => {
                self.pd_controller
                    .send_response(controller::Response::Lpm(lpm::Response::Err(
                        PdError::UnrecognizedCommand,
                    )))
                    .await;
            }
        }
    }
}
