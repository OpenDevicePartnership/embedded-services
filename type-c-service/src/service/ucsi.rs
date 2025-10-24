use embedded_services::warn;
use embedded_usb_pd::PdError;
use embedded_usb_pd::ucsi::cci::{Cci, GlobalCci};
use embedded_usb_pd::ucsi::lpm::get_connector_status::ConnectorStatusChange;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::ppm::state_machine::{
    GlobalInput as PpmInput, GlobalOutput as PpmOutput, GlobalStateMachine as StateMachine, InvalidTransition,
};
use embedded_usb_pd::ucsi::{GlobalCommand, ResponseData, lpm, ppm};

use super::*;

/// UCSI state
#[derive(Default)]
pub(super) struct State {
    /// PPM state machine
    ppm_state_machine: StateMachine,
    /// Currently enabled notifications
    notifications_enabled: NotificationEnable,
    /// Queued pending port notifications
    pending_ports: heapless::Deque<GlobalPortId, MAX_SUPPORTED_PORTS>,
}

impl<'a> Service<'a> {
    /// PPM reset implementation
    async fn process_ppm_reset(&self, state: &mut State) {
        debug!("Resetting PPM");
        state.notifications_enabled = NotificationEnable::default();
        state.pending_ports.clear();
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
        debug!("Processing LPM command: {:?}", command);
        if matches!(command.operation(), lpm::CommandData::GetConnectorCapability) {
            // Override the capabilities if present in the config
            if let Some(capabilities) = &self.config.ucsi_port_capabilities {
                Ok(Some(lpm::ResponseData::GetConnectorCapability(*capabilities)))
            } else {
                self.context.execute_ucsi_command(*command).await
            }
        } else {
            self.context.execute_ucsi_command(*command).await
        }
    }

    /// Upate the CCI connector change field based on the current pending port
    fn set_cci_connector_change(&self, state: &mut State, cci: &mut GlobalCci) {
        if let Some(current_port) = state.pending_ports.front() {
            // UCSI connector numbers are 1-based
            cci.set_connector_change(GlobalPortId(current_port.0 + 1));
        } else {
            // 0 is used to indicate no pending connector changes
            cci.set_connector_change(GlobalPortId(0));
        }
    }

    /// Acknowledge the current connector change and move to the next if present
    async fn ack_connector_change(&self, state: &mut State, cci: &mut GlobalCci) {
        // Pop the just acknowledged port and move to the next if present
        if let Some(_current_port) = state.pending_ports.pop_front() {
            if let Some(next_port) = state.pending_ports.front() {
                debug!("ACK_CCI processed, next pending port: {:?}", next_port);
                self.context
                    .broadcast_message(comms::CommsMessage::UcsiCci(comms::UsciChangeIndicator {
                        port: *next_port,
                        // False here because the OPM gets notified by the CCI, don't need a separate notification
                        notify_opm: false,
                    }))
                    .await;
            } else {
                debug!("ACK_CCI processed, no more pending ports");
            }
        } else {
            warn!("Received ACK_CCI with no pending connector changes");
        }

        self.set_cci_connector_change(state, cci);
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
                        self.set_cci_connector_change(state, &mut response.cci);
                        return response;
                    }
                    PpmOutput::AckComplete(ack) => {
                        response.notify_opm = state.notifications_enabled.cmd_complete();
                        if ack.command_complete() {
                            response.cci.set_ack_command(true);
                        }

                        if ack.connector_change() {
                            self.ack_connector_change(state, &mut response.cci).await;
                        }

                        return response;
                    }
                    PpmOutput::ResetComplete => {
                        // Resets don't follow the normal command execution flow
                        // So do any reset processing here
                        self.process_ppm_reset(state).await;
                        // Don't notify OPM because it'll poll
                        response.notify_opm = false;
                        response.cci = Cci::new_reset_complete();
                        self.set_cci_connector_change(state, &mut response.cci);
                        return response;
                    }
                    PpmOutput::OpmNotifyBusy => {
                        // Notify if notifications are enabled in general
                        response.notify_opm = !state.notifications_enabled.is_empty();
                        response.cci.set_busy(true);
                        self.set_cci_connector_change(state, &mut response.cci);
                        return response;
                    }
                },
                None => {
                    // No output from PPM state machine, nothing specific to send back
                    response.notify_opm = false;
                    response.cci = Cci::default();
                    response.data = Ok(None);
                    self.set_cci_connector_change(state, &mut response.cci);
                    return response;
                }
            }
        }
    }

    /// Convert from general PD events into UCSI-specific events
    pub(super) async fn generate_ucsi_event(&self, port_id: GlobalPortId, port_event: PortStatusChanged) {
        let state = &mut self.state.lock().await.ucsi;
        let mut ucsi_event = ConnectorStatusChange::default();

        ucsi_event.set_connect_change(port_event.plug_inserted_or_removed());
        ucsi_event.set_power_direction_changed(port_event.power_swap_completed());
        ucsi_event.set_pd_reset_complete(port_event.pd_hard_reset());

        if port_event.data_swap_completed() || port_event.alt_mode_entered() {
            ucsi_event.set_connector_partner_changed(true);
        }

        if port_event.new_power_contract_as_consumer() || port_event.new_power_contract_as_provider() {
            ucsi_event.set_negotiated_power_level_change(true);
            ucsi_event.set_power_op_mode_change(true);
            ucsi_event.set_external_supply_change(true);
            ucsi_event.set_power_direction_changed(true);
            ucsi_event.set_battery_charging_status_change(true);
        }

        if ucsi_event.filter_enabled(state.notifications_enabled).is_none() {
            trace!("{:?}: event received, but no UCSI notifications enabled", port_id);
            return;
        }

        if state.pending_ports.iter().any(|pending| *pending == port_id) {
            // Already have a pending event for this port, don't need to process it twice
            return;
        }

        // Only notifiy the OPM if we don't have any pending events
        // Once the OPM starts processing events, the next pending port will be sent as part
        // of the CCI response to the ACK_CC_CI command. See [`Self::set_cci_connector_change`]
        let notify_opm = state.pending_ports.is_empty();
        if state.pending_ports.push_back(port_id).is_ok() {
            self.context
                .broadcast_message(comms::CommsMessage::UcsiCci(comms::UsciChangeIndicator {
                    port: port_id,
                    notify_opm,
                }))
                .await;
        } else {
            // This shouldn't happen because we have a single slot per port
            // Would likely indicate that an invalid port ID got in somehow
            error!("Pending UCSI events overflow");
        }
    }
}
