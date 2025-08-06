use embedded_usb_pd::ucsi::cci::Cci;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::ppm::state_machine::{
    Input as PpmInput, Output as PpmOutput, State as PpmState, StateMachine,
};
use embedded_usb_pd::ucsi::{ppm, Command, Response, ResponseData};
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
    async fn process_set_notification_enable(&self, state: &mut State, enable: NotificationEnable) -> Response {
        debug!("Set Notification Enable: {:?}", enable);
        state.notifications_enabled = enable;
        Cci::new_cmd_complete().into()
    }

    /// PPM reset implementation
    fn process_ppm_reset(&self) -> (Response, Option<PpmInput>) {
        debug!("PPM reset");
        (Cci::new_reset_complete().into(), Some(PpmInput::Reset))
    }

    /// PPM get capabilities implementation
    fn process_get_capabilities(&self) -> (Response, Option<PpmInput>) {
        debug!("Get PPM capabilities: {:?}", self.config.ucsi_capabilities);
        (
            Response {
                cci: Cci::new_cmd_complete(),
                // TODO: pull num connectors from service
                data: Some(ResponseData::PpmResponse(ppm::Response::GetCapability(
                    self.config.ucsi_capabilities,
                ))),
            },
            Some(PpmInput::CommandImmediate),
        )
    }

    /// Process a command when the PPM state machine is idle and notifications are disabled
    async fn process_ppm_state_idle_disabled(
        &self,
        state: &mut State,
        command: &Command,
    ) -> Result<(Response, Option<PpmInput>), PdError> {
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
        command: &Command,
    ) -> Result<(Response, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::SetNotificationEnable(enable)) => Ok((
                self.process_set_notification_enable(state, enable.notification_enable)
                    .await,
                Some(PpmInput::CommandImmediate),
            )),
            Command::PpmCommand(ppm::Command::GetCapability) => Ok(self.process_get_capabilities()),
            // TODO: implement other commands
            _ => Err(PdError::UnrecognizedCommand),
        }
    }

    /// Process a command when the PPM state machine is busy
    async fn process_ppm_state_busy(&self, command: &Command) -> Result<(Response, Option<PpmInput>), PdError> {
        match command {
            // Reset is the only command that can be processed in busy state
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            _ => Err(PdError::InvalidMode),
        }
    }

    /// Process a command when the PPM state machine is processing a command
    async fn process_ppm_state_processing_command(
        &self,
        command: &Command,
    ) -> Result<(Response, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            // Cancel the current command
            Command::PpmCommand(ppm::Command::Cancel) => Ok((Cci::new_cmd_complete().into(), Some(PpmInput::Cancel))),
            _ => Err(PdError::InvalidMode),
        }
    }

    /// Process a command when the PPM state machine is waiting for a command complete ack
    async fn process_ppm_state_wait_for_command_complete_ack(
        &self,
        command: &Command,
    ) -> Result<(Response, Option<PpmInput>), PdError> {
        match command {
            Command::PpmCommand(ppm::Command::PpmReset) => Ok(self.process_ppm_reset()),
            Command::PpmCommand(ppm::Command::AckCcCi(args)) => {
                if args.ack.command_complete() {
                    Ok((Cci::new_cmd_complete().into(), Some(PpmInput::CommandCompleteAck)))
                } else {
                    // Still waiting for ack
                    Ok((Cci::default().into(), None))
                }
            }
            // All other commands are invalid in this state
            _ => Err(PdError::InvalidMode),
        }
    }

    /// Process a command when the PPM state machine is waiting for an async event ack
    async fn process_ppm_state_wait_for_async_event_ack(
        &self,
        command: &Command,
    ) -> Result<(Response, Option<PpmInput>), PdError> {
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
            _ => Err(PdError::InvalidMode),
        }
    }

    /// Process an external UCSI command
    pub(super) async fn process_ucsi_command(&self, command: &Command) -> Result<external::UcsiResponse, PdError> {
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
                    PpmOutput::OpmNotifyReset => {
                        // Always notify OPM on reset
                        notify_opm = true;
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
                        response.cci.set_connector_change(0);
                    }
                }
            }
        }

        Ok(external::UcsiResponse { notify_opm, response })
    }
}
