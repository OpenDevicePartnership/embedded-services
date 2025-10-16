use core::mem;
use embedded_services::type_c::event::{PortPending, PortPendingIter};
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
    // Pending connector changes
    pending_ports: PortPending,
    /// Iterator to implement round robin over pending port events
    pending_ports_iter: Option<PortPendingIter>,
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
        if let Some(current_port) = state.pending_ports_iter.and_then(|mut iter| iter.next()) {
            // UCSI connector numbers are 1-based
            cci.set_connector_change(GlobalPortId(current_port as u8 + 1));
        } else {
            cci.set_connector_change(GlobalPortId(0));
        }
    }

    /// Start a new round robin over pending ports, notifying OPM if requested
    async fn start_connector_changed_notify(&self, state: &mut State, notify_opm: bool) {
        let mut iter = mem::take(&mut state.pending_ports).into_iter();

        state.pending_ports_iter = Some(iter);
        if let Some(port_id) = iter.next() {
            // Notify OPM if requested
            self.context
                .broadcast_message(comms::CommsMessage::UcsiCci(comms::UcsiCiMessage {
                    port: GlobalPortId(port_id as u8),
                    notify_opm,
                }))
                .await;
        }
    }

    /// Acknowledge the current connector change and move to the next if present
    async fn ack_connector_change(&self, state: &mut State, cci: &mut GlobalCci) {
        if let Some(current_port) = state.pending_ports_iter.as_mut().and_then(|iter| iter.next()) {
            state.pending_ports.clear_port(current_port);

            if let Some(next_port) = state.pending_ports_iter.and_then(|mut iter| iter.next()) {
                // More pending ports
                debug!("ACK_CCI processed, next pending port: {}", next_port);
                self.context
                    .broadcast_message(comms::CommsMessage::UcsiCci(comms::UcsiCiMessage {
                        port: GlobalPortId(next_port as u8),
                        // False here because the OPM gets notified by the CCI, don't need a separate notification
                        notify_opm: false,
                    }))
                    .await;
            } else if !state.pending_ports.is_none() {
                // More pending ports, restart the round robin
                debug!("ACK_CCI processed, restarting UCSI event round robin");
                self.start_connector_changed_notify(state, false).await;
            } else {
                // No more pending ports, end the round robin
                debug!("ACK_CCI processed, no more pending ports");
                state.pending_ports_iter = None;
            }
        } else {
            // Got an ACK_CCI with no pending ports, fail gracefully and produce a warning
            warn!("Received ACK_CCI with no pending connector changes");
            state.pending_ports_iter = None;
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

    pub(super) async fn process_ucsi_event(&self, port_id: GlobalPortId, port_event: PortStatusChanged) {
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

        if !ucsi_event.filter_enabled(state.notifications_enabled).is_none() {
            state.pending_ports.pend_port(port_id.0 as usize);
            debug!(
                "Port{}: Queuing UCSI connector change event: {:?}",
                port_id.0, ucsi_event
            );
        }

        if state.pending_ports_iter.is_none() && !state.pending_ports.is_none() {
            // Start a new round robin pass and broadcast the initial event
            // Subsequent events will be broadcast when ACK_CC_CI is processed
            self.start_connector_changed_notify(state, state.notifications_enabled.connect_change())
                .await;
        }
    }
}
