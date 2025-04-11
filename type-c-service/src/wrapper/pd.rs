use embedded_usb_pd::ucsi::lpm;

use super::*;

impl<const N: usize, C: Controller> ControllerWrapper<'_, N, C> {
    /// Handle a port command
    async fn process_port_command(&self, controller: &mut C, command: controller::PortCommand) {
        let response = match command.data {
            controller::PortCommandData::PortStatus => match controller.get_port_status(LocalPortId(0)).await {
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
        };

        self.pd_controller
            .send_response(controller::Response::Port(response))
            .await;
    }

    async fn process_controller_command(&self, _controller: &mut C, command: controller::InternalCommandData) {
        let response = match command {
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
