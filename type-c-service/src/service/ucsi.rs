use embedded_usb_pd::ucsi::cci::Cci;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::ppm::state_machine::{
    GlobalInput as PpmInput, GlobalOutput as PpmOutput, GlobalStateMachine as StateMachine, InvalidTransition,
};
use embedded_usb_pd::ucsi::{lpm, ppm, GlobalCommand, ResponseData};
use embedded_usb_pd::PdError;

use super::*;

/// UCSI state
#[derive(Default)]
pub(super) struct State {
    /// PPM state machine
    ppm_state_machine: StateMachine,
    /// Currently enabled notifications
    notifications_enabled: NotificationEnable,
}

impl<'a> Service<'a> {
    /// PPM reset implementation
    async fn process_ppm_reset(&self, state: &mut State) {
        debug!("Resetting PPM");
        state.notifications_enabled = NotificationEnable::default();
    }

    /// Set notification enable implementation
    async fn process_set_notification_enable(&self, state: &mut State, enable: NotificationEnable) {
        debug!("Set Notification Enable: {:?}", enable);
        state.notifications_enabled = enable;
    }

    /// PPM get capabilities implementation
    async fn process_get_capabilities(&self) -> ppm::ResponseData {
        debug!("Get PPM capabilities: {:?}", self.config.ucsi_capabilities);
        let mut capabilities = self.config.ucsi_capabilities;
        capabilities.num_connectors = external::get_num_ports().await as u8;
        ppm::ResponseData::GetCapability(capabilities)
    }

    async fn process_ppm_command(
        &self,
        state: &mut State,
        command: &ucsi::ppm::Command,
    ) -> Result<Option<ppm::ResponseData>, PdError> {
        match command {
            ppm::Command::SetNotificationEnable(enable) => {
                self.process_set_notification_enable(state, enable.notification_enable)
                    .await;
                Ok(None)
            }
            ppm::Command::GetCapability => Ok(Some(self.process_get_capabilities().await)),
            _ => Ok(None), // Other commands are currently no-ops
        }
    }

    async fn process_lpm_command(
        &self,
        command: &ucsi::lpm::GlobalCommand,
    ) -> Result<Option<lpm::ResponseData>, PdError> {
        match command.operation {
            lpm::CommandData::GetConnectorCapability => Ok(Some(lpm::ResponseData::GetConnectorCapability(
                //TODO: Send command to controller
                *lpm::get_connector_capability::ResponseData::default()
                    .set_operation_mode(
                        *OperationModeFlags::default()
                            .set_drp(true)
                            .set_usb2(true)
                            .set_usb3(true),
                    )
                    .set_consumer(true)
                    .set_provider(true)
                    .set_swap_to_dfp(true)
                    .set_swap_to_snk(true)
                    .set_swap_to_src(true),
            ))),
            lpm::CommandData::GetConnectorStatus => Ok(
                //TODO: Send command to controller
                Some(lpm::ResponseData::GetConnectorStatus(
                    lpm::get_connector_status::ResponseData::default(),
                )),
            ),
            // TODO: implement all other LPM commands
            rest => {
                error!("Unsupported command received: {:?}", rest);
                Err(PdError::UnrecognizedCommand)
            }
        }
    }

    /// Process an external UCSI command
    pub(super) async fn process_ucsi_command(&self, command: &GlobalCommand) -> external::UcsiResponse {
        let state = &mut self.state.lock().await.ucsi;
        let mut next_input = Some(PpmInput::Command(command));
        let mut response: external::UcsiResponse = external::UcsiResponse {
            notify_opm: false,
            cci: Cci::default(),
            data: Ok(None),
        };

        // Loop to simplify the processing of commands
        // Executing a command requires two passes through the state machine
        // Using a loop allows all logic to be centralized
        loop {
            if next_input.is_none() {
                error!("Unexpected end of state machine processing");
                return external::UcsiResponse {
                    notify_opm: true,
                    cci: Cci::new_error(),
                    data: Err(PdError::InvalidMode),
                };
            }

            let output = state.ppm_state_machine.consume(next_input.take().unwrap());
            if let Err(e @ InvalidTransition { .. }) = &output {
                error!("PPM state machine transition failed: {:#?}", e);
                return external::UcsiResponse {
                    notify_opm: true,
                    cci: Cci::new_error(),
                    data: Err(PdError::Failed),
                };
            }

            match output.unwrap() {
                Some(ppm_output) => match ppm_output {
                    PpmOutput::ExecuteCommand(command) => {
                        // Queue up the next input to complete the command execution flow
                        next_input = Some(PpmInput::CommandComplete);
                        match command {
                            ucsi::GlobalCommand::PpmCommand(ppm_command) => {
                                response.data = self
                                    .process_ppm_command(state, ppm_command)
                                    .await
                                    .map(|inner| inner.map(ResponseData::Ppm));
                            }
                            ucsi::GlobalCommand::LpmCommand(lpm_command) => {
                                response.data = self
                                    .process_lpm_command(lpm_command)
                                    .await
                                    .map(|inner| inner.map(ResponseData::Lpm));
                            }
                        }

                        // Don't return yet, need to inform state machine that command is complete
                    }
                    PpmOutput::OpmNotifyCommandComplete => {
                        response.notify_opm = state.notifications_enabled.cmd_complete();
                        response.cci.set_cmd_complete(true);
                        response.cci.set_error(response.data.is_err());
                        return response;
                    }
                    PpmOutput::OpmNotifyAckComplete => {
                        // This is really a command complete, but it's signaled differently in the CCI
                        response.notify_opm = state.notifications_enabled.cmd_complete();
                        response.cci.set_ack_command(true);
                        return response;
                    }
                    PpmOutput::ResetComplete => {
                        // Resets don't follow the normal command execution flow
                        // So do any reset processing here
                        self.process_ppm_reset(state).await;
                        // Don't notify OPM because it'll poll
                        response.notify_opm = false;
                        response.cci = Cci::new_reset_complete();
                        return response;
                    }
                    PpmOutput::OpmNotifyBusy => {
                        // Notify if notifications are enabled in general
                        response.notify_opm = !state.notifications_enabled.is_empty();
                        response.cci.set_busy(true);
                        return response;
                    }
                    PpmOutput::OpmNotifyAsyncEvent => {
                        response.notify_opm = state.notifications_enabled.connect_change();
                        // TODO: use actual port
                        response.cci.set_connector_change(GlobalPortId(0));
                        return response;
                    }
                },
                None => {
                    // No output from PPM state machine, nothing specific to send back
                    response.notify_opm = false;
                    response.cci = Cci::default();
                    response.data = Ok(None);
                    return response;
                }
            }
        }
    }
}
