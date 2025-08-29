use embedded_usb_pd::ucsi::cci::Cci;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::ppm::state_machine::{
    Input as PpmInput, Output as PpmOutput, State as PpmState, StateMachine,
};
use embedded_usb_pd::ucsi::{lpm, ppm, Command, GlobalCommand, GlobalResponse, ResponseData};
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
    /// Set notification enable implementation
    async fn process_set_notification_enable(&self, state: &mut State, enable: NotificationEnable) -> GlobalResponse {
        debug!("Set Notification Enable: {:?}", enable);
        state.notifications_enabled = enable;
        Cci::new_cmd_complete().into()
    }

    /// PPM reset implementation
    fn process_ppm_reset(&self) -> (GlobalResponse, Option<PpmInput>) {
        debug!("PPM reset");
        (Cci::new_reset_complete().into(), Some(PpmInput::Reset))
    }

    /// PPM get capabilities implementation
    async fn process_get_capabilities(&self) -> (GlobalResponse, Option<PpmInput>) {
        debug!("Get PPM capabilities: {:?}", self.config.ucsi_capabilities);
        let mut capabilities = self.config.ucsi_capabilities;
        capabilities.num_connectors = external::get_num_ports().await as u8;
        (
            GlobalResponse {
                cci: Cci::new_cmd_complete(),
                data: Some(ResponseData::PpmResponse(ppm::ResponseData::GetCapability(
                    capabilities,
                ))),
            },
            Some(PpmInput::CommandImmediate),
        )
    }

    /// Process a command when the PPM state machine is idle and notifications are disabled
    async fn process_ppm_state_idle_disabled(
        &self,
        state: &mut State,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::SetNotificationEnable(enable)) => Ok((
                self.process_set_notification_enable(state, enable.notification_enable)
                    .await,
                Some(PpmInput::NotificationEnabled),
            )),
            _ => Err(PdError::InvalidMode),
        }
    }

    /// Process a command when the PPM state machine is idle and notifications are enabled
    async fn process_ppm_state_idle_enabled(
        &self,
        state: &mut State,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::SetNotificationEnable(enable)) => Ok((
                self.process_set_notification_enable(state, enable.notification_enable)
                    .await,
                Some(PpmInput::CommandImmediate),
            )),
            Command::PpmCommand(ppm::Command::GetCapability) => Ok(self.process_get_capabilities().await),
            Command::LpmCommand(lpm::GlobalCommand { port: _, operation }) => match operation {
                lpm::CommandData::GetConnectorCapability => Ok((
                    //TODO: Send command to controller
                    GlobalResponse {
                        cci: Cci::new_cmd_complete(),
                        data: Some(ResponseData::LpmResponse(lpm::ResponseData::GetConnectorCapability(
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
                    },
                    Some(PpmInput::CommandImmediate),
                )),
                lpm::CommandData::GetConnectorStatus => Ok((
                    //TODO: Send command to controller
                    GlobalResponse {
                        cci: Cci::new_cmd_complete(),
                        data: Some(ResponseData::LpmResponse(lpm::ResponseData::GetConnectorStatus(
                            lpm::get_connector_status::ResponseData::default(),
                        ))),
                    },
                    Some(PpmInput::CommandImmediate),
                )),
                // TODO: implement all other LPM commands
                rest => {
                    error!("Unsupported command received: {:?}", rest);
                    Err(PdError::UnrecognizedCommand)
                }
            },
            // TODO: implement other commands
            _ => {
                error!("Invalid command received: {:?}", command);
                Err(PdError::InvalidMode)
            }
        }
    }

    /// Process a command when the PPM state machine is busy
    async fn process_ppm_state_busy(
        &self,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            // Reset is the only command that can be processed in busy state
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            _ => {
                error!("Invalid command received while busy: {:?}", command);
                Err(PdError::InvalidMode)
            }
        }
    }

    /// Process a command when the PPM state machine is processing a command
    async fn process_ppm_state_processing_command(
        &self,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            // Cancel the current command
            Command::PpmCommand(ppm::Command::Cancel) => Ok((Cci::new_cmd_complete().into(), Some(PpmInput::Cancel))),
            _ => {
                error!("Invalid command received while processing command: {:?}", command);
                Err(PdError::InvalidMode)
            }
        }
    }

    /// Process a command when the PPM state machine is waiting for a command complete ack
    async fn process_ppm_state_wait_for_command_complete_ack(
        &self,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::AckCcCi(args)) => {
                if args.ack.command_complete() {
                    Ok((
                        (*Cci::default().set_ack_command(true)).into(),
                        Some(PpmInput::CommandCompleteAck),
                    ))
                } else {
                    // Still waiting for ack
                    Ok((Cci::default().into(), None))
                }
            }
            // All other commands are invalid in this state
            _ => {
                error!(
                    "Invalid command received while waiting for command complete ack: {:?}",
                    command
                );
                Err(PdError::InvalidMode)
            }
        }
    }

    /// Process a command when the PPM state machine is waiting for an async event ack
    async fn process_ppm_state_wait_for_async_event_ack(
        &self,
        command: &GlobalCommand,
    ) -> Result<(GlobalResponse, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::AckCcCi(args)) => {
                if args.ack.connector_change() {
                    Ok((Cci::new_cmd_complete().into(), Some(PpmInput::AsyncEventAck)))
                } else {
                    // Still waiting for ack
                    Ok((Cci::default().into(), None))
                }
            }
            // All other commands are invalid in this state
            _ => {
                error!(
                    "Invalid command received while waiting for async event ack: {:?}",
                    command
                );
                Err(PdError::InvalidMode)
            }
        }
    }

    /// Process an external UCSI command
    pub(super) async fn process_ucsi_command(
        &self,
        command: &GlobalCommand,
    ) -> Result<external::UcsiResponse, PdError> {
        let state = &mut self.state.lock().await.ucsi;
        let (mut response, ppm_input) = match state.ppm_state_machine.state() {
            PpmState::Idle(false) => self.process_ppm_state_idle_disabled(state, command).await,
            PpmState::Idle(true) => self.process_ppm_state_idle_enabled(state, command).await,
            PpmState::Busy(_) => self.process_ppm_state_busy(command).await,
            PpmState::ProcessingCommand => self.process_ppm_state_processing_command(command).await,
            PpmState::WaitForCommandCompleteAck => self.process_ppm_state_wait_for_command_complete_ack(command).await,
            PpmState::WaitForAsyncEventAck => self.process_ppm_state_wait_for_async_event_ack(command).await,
        }?;

        let mut notify_opm = false;
        // Feed any PPM input to the state machine
        if let Some(ppm_input) = ppm_input {
            // Process state machine output
            if let Some(output) = state.ppm_state_machine.consume(ppm_input).map_err(|e| {
                error!("PPM state machine transition failed: {:#?}", e);
                PdError::Failed
            })? {
                match output {
                    PpmOutput::OpmNotifyCommandComplete => {
                        notify_opm = state.notifications_enabled.cmd_complete();
                        response.cci.set_cmd_complete(true);
                    }
                    PpmOutput::OpmNotifyAckComplete => {
                        // This is really a command complete, but it's signaled differently in the CCI
                        notify_opm = state.notifications_enabled.cmd_complete();
                        response.cci.set_ack_command(true);
                    }
                    PpmOutput::OpmNotifyReset => {
                        response.cci.set_reset_complete(true);
                    }
                    PpmOutput::OpmNotifyBusy => {
                        // Notify if notifications are enabled in general
                        notify_opm = !state.notifications_enabled.is_empty();
                        response.cci.set_busy(true);
                    }
                    PpmOutput::OpmNotifyAsyncEvent => {
                        notify_opm = state.notifications_enabled.connect_change();
                        // TODO: use real port
                        response.cci.set_connector_change(GlobalPortId(0));
                    }
                }
            }
        }

        Ok(external::UcsiResponse { notify_opm, response })
    }
}
