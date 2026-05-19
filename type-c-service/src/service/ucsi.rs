use embedded_services::sync::Lockable;
use embedded_services::warn;
use embedded_usb_pd::ucsi::cci::{Cci, GlobalCci};
use embedded_usb_pd::ucsi::lpm::get_connector_status::{BatteryChargingCapabilityStatus, ConnectorStatusChange};
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::ppm::state_machine::{
    GlobalInput as PpmInput, GlobalOutput as PpmOutput, GlobalStateMachine as StateMachine, InvalidTransition,
};
use embedded_usb_pd::ucsi::{GlobalCommand, ResponseData, lpm, ppm};
use embedded_usb_pd::{PdError, PowerRole};
use type_c_interface::service::event::{Event, UsciChangeIndicatorData};
use type_c_interface::ucsi::Lpm as _;

use super::*;

const MAX_SUPPORTED_PORTS: usize = 4;

/// UCSI command response
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UcsiResponse {
    /// Notify the OPM, the function call
    pub notify_opm: bool,
    /// Response CCI
    pub cci: GlobalCci,
    /// UCSI response data
    pub data: Result<Option<ucsi::ResponseData>, PdError>,
}

/// UCSI state
#[derive(Default)]
pub(super) struct State {
    /// PPM state machine
    pub ppm_state_machine: StateMachine,
    /// Currently enabled notifications
    pub notifications_enabled: NotificationEnable,
    /// Queued pending port notifications
    pub pending_ports: heapless::Deque<GlobalPortId, MAX_SUPPORTED_PORTS>,
    /// Ports that have a valid battery charging status capability
    ///
    /// We provide a battery charging status only after the port has negotiated power.
    /// This prevents the port from temporarily reporting slow or no charging before the contract has finalized.
    pub valid_battery_charging_capability: heapless::index_set::FnvIndexSet<GlobalPortId, MAX_SUPPORTED_PORTS>,
    /// PSU connected
    pub psu_connected: bool,
}

impl<'port, Reg: Registration<'port>> Service<'port, Reg> {
    /// PPM reset implementation
    fn process_ppm_reset(&mut self) {
        debug!("Resetting PPM");
        self.ucsi.notifications_enabled = NotificationEnable::default();
        self.ucsi.pending_ports.clear();
        self.ucsi.valid_battery_charging_capability.clear();
    }

    /// Set notification enable implementation
    fn process_set_notification_enable(&mut self, enable: NotificationEnable) {
        debug!("Set Notification Enable: {:?}", enable);
        self.ucsi.notifications_enabled = enable;
    }

    /// PPM get capabilities implementation
    fn process_get_capabilities(&self) -> ppm::ResponseData {
        debug!("Get PPM capabilities: {:?}", self.config.ucsi_capabilities);
        let mut capabilities = self.config.ucsi_capabilities;
        capabilities.num_connectors = self.registration.ports().len() as u8;
        ppm::ResponseData::GetCapability(capabilities)
    }

    fn process_ppm_command(&mut self, command: &ucsi::ppm::Command) -> Result<Option<ppm::ResponseData>, PdError> {
        match command {
            ppm::Command::SetNotificationEnable(enable) => {
                self.process_set_notification_enable(enable.notification_enable);
                Ok(None)
            }
            ppm::Command::GetCapability => Ok(Some(self.process_get_capabilities())),
            _ => Ok(None), // Other commands are currently no-ops
        }
    }

    /// Determine the battery charging capability status for the given port
    fn determine_battery_charging_capability_status(
        &self,
        port_id: GlobalPortId,
        port_status: &PortStatus,
    ) -> Option<BatteryChargingCapabilityStatus> {
        if port_status.power_role == PowerRole::Sink {
            if self.ucsi.valid_battery_charging_capability.contains(&port_id) && !self.ucsi.psu_connected {
                // Only run this logic when no PSU is attached to prevent excessive notifications
                // when new type-C PSUs are attached
                let power_mw = port_status
                    .available_sink_contract
                    .map(|contract| contract.max_power_mw())
                    .unwrap_or(0);

                Some(self.config.ucsi_battery_charging_config.status_of(power_mw))
            } else {
                // Report normal charging until something changes
                Some(BatteryChargingCapabilityStatus::Nominal)
            }
        } else {
            // This field only applies to sinks
            None
        }
    }

    async fn process_lpm_command(
        &mut self,
        command: &ucsi::lpm::GlobalCommand,
    ) -> Result<Option<lpm::ResponseData>, PdError> {
        debug!("Processing LPM command: {:?}", command);
        let mut port = self.lookup_port(command.port())?.lock().await;
        let local_port_id = self
            .registration
            .ucsi_local_port_id(command.port())
            .ok_or(PdError::InvalidPort)?;
        let local_command = ucsi::lpm::LocalCommand::new(local_port_id, command.operation());

        match command.operation() {
            lpm::CommandData::GetConnectorCapability => {
                // Override the capabilities if present in the config
                if let Some(capabilities) = &self.config.ucsi_port_capabilities {
                    Ok(Some(lpm::ResponseData::GetConnectorCapability(*capabilities)))
                } else {
                    port.execute_lpm_command(local_command).await
                }
            }
            lpm::CommandData::GetConnectorStatus => {
                let mut response = port.execute_lpm_command(local_command).await;
                if let Ok(Some(lpm::ResponseData::GetConnectorStatus(lpm::get_connector_status::ResponseData {
                    status_change: ref mut states_change,
                    status:
                        Some(lpm::get_connector_status::ConnectedStatus {
                            ref mut battery_charging_status,
                            ..
                        }),
                    ..
                }))) = response
                {
                    let port_status = port.get_port_status().await?;
                    *battery_charging_status =
                        self.determine_battery_charging_capability_status(command.port(), &port_status);
                    states_change.set_battery_charging_status_change(battery_charging_status.is_some());
                }

                response
            }
            _ => port.execute_lpm_command(local_command).await,
        }
    }

    /// Update the CCI connector change field based on the current pending port
    fn set_cci_connector_change(&self, cci: &mut GlobalCci) {
        if let Some(current_port) = self.ucsi.pending_ports.front() {
            // UCSI connector numbers are 1-based
            cci.set_connector_change(GlobalPortId(current_port.0 + 1));
        } else {
            // 0 is used to indicate no pending connector changes
            cci.set_connector_change(GlobalPortId(0));
        }
    }

    /// Acknowledge the current connector change and move to the next if present
    async fn ack_connector_change(&mut self, cci: &mut GlobalCci) {
        // Pop the just acknowledged port and move to the next if present
        let Some(_current_port) = self.ucsi.pending_ports.pop_front() else {
            warn!("Received ACK_CCI with no pending connector changes");
            return;
        };

        let Some(next_port) = self.ucsi.pending_ports.front() else {
            debug!("ACK_CCI processed, no more pending ports");
            return;
        };

        debug!("ACK_CCI processed, next pending port: {:?}", next_port);
        let Ok(port) = self.lookup_port(*next_port) else {
            error!("Invalid port ID in pending ports: {:?}", next_port);
            return;
        };

        self.broadcast_event(Event {
            port,
            event: EventData::UsciChangeIndicator(UsciChangeIndicatorData {
                port: *next_port,
                // False here because the OPM gets notified by the CCI, don't need a separate notification
                notify_opm: false,
            }),
        })
        .await;

        self.set_cci_connector_change(cci);
    }

    /// Process a UCSI command
    pub async fn process_ucsi_command(&mut self, command: &GlobalCommand) -> UcsiResponse {
        let mut next_input = Some(PpmInput::Command(command));
        let mut response = UcsiResponse {
            notify_opm: false,
            cci: Cci::default(),
            data: Ok(None),
        };

        // Loop to simplify the processing of commands
        // Executing a command requires two passes through the state machine
        // Using a loop allows all logic to be centralized
        loop {
            let output = if let Some(next_input) = next_input.take() {
                self.ucsi.ppm_state_machine.consume(next_input)
            } else {
                error!("Unexpected end of state machine processing");
                return UcsiResponse {
                    notify_opm: true,
                    cci: Cci::new_error(),
                    data: Err(PdError::InvalidMode),
                };
            };

            let output = match &output {
                Ok(output) => output,
                Err(e @ InvalidTransition { .. }) => {
                    error!("PPM state machine transition failed: {:#?}", e);
                    return UcsiResponse {
                        notify_opm: true,
                        cci: Cci::new_error(),
                        data: Err(PdError::Failed),
                    };
                }
            };

            match output {
                Some(ppm_output) => match ppm_output {
                    PpmOutput::ExecuteCommand(command) => {
                        // Queue up the next input to complete the command execution flow
                        next_input = Some(PpmInput::CommandComplete);
                        match command {
                            ucsi::GlobalCommand::PpmCommand(ppm_command) => {
                                response.data = self
                                    .process_ppm_command(ppm_command)
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
                        response.notify_opm = self.ucsi.notifications_enabled.cmd_complete();
                        response.cci.set_cmd_complete(true);
                        response.cci.set_error(response.data.is_err());
                        self.set_cci_connector_change(&mut response.cci);
                        return response;
                    }
                    PpmOutput::AckComplete(ack) => {
                        response.notify_opm = self.ucsi.notifications_enabled.cmd_complete();
                        if ack.command_complete() {
                            response.cci.set_ack_command(true);
                        }

                        if ack.connector_change() {
                            self.ack_connector_change(&mut response.cci).await;
                        }

                        return response;
                    }
                    PpmOutput::ResetComplete => {
                        // Resets don't follow the normal command execution flow
                        // So do any reset processing here
                        self.process_ppm_reset();
                        // Don't notify OPM because it'll poll
                        response.notify_opm = false;
                        response.cci = Cci::new_reset_complete();
                        self.set_cci_connector_change(&mut response.cci);
                        return response;
                    }
                    PpmOutput::OpmNotifyBusy => {
                        // Notify if notifications are enabled in general
                        response.notify_opm = !self.ucsi.notifications_enabled.is_empty();
                        response.cci.set_busy(true);
                        self.set_cci_connector_change(&mut response.cci);
                        return response;
                    }
                },
                None => {
                    // No output from PPM state machine, nothing specific to send back
                    response.notify_opm = false;
                    response.cci = Cci::default();
                    response.data = Ok(None);
                    self.set_cci_connector_change(&mut response.cci);
                    return response;
                }
            }
        }
    }

    /// Handle PD port events, update UCSI state, and generate corresponding UCSI notifications
    pub(super) async fn handle_ucsi_port_event(
        &mut self,
        port: &'port Reg::Port,
        port_id: GlobalPortId,
        port_event: PortStatusEventBitfield,
        port_status: &PortStatus,
    ) {
        let mut ucsi_event = ConnectorStatusChange::default();

        ucsi_event.set_connect_change(port_event.plug_inserted_or_removed());
        ucsi_event.set_power_direction_changed(port_event.power_swap_completed());
        ucsi_event.set_pd_reset_complete(port_event.pd_hard_reset());

        if port_event.data_swap_completed() || port_event.alt_mode_entered() {
            ucsi_event.set_connector_partner_changed(true);
        }

        if port_event.new_power_contract_as_consumer()
            || port_event.new_power_contract_as_provider()
            || port_event.sink_ready()
        {
            ucsi_event.set_negotiated_power_level_change(true);
            ucsi_event.set_power_op_mode_change(true);
            ucsi_event.set_external_supply_change(true);
            ucsi_event.set_power_direction_changed(true);
            ucsi_event.set_battery_charging_status_change(true);

            // Power negotiation completed, battery charging capability status is now valid
            if self.ucsi.valid_battery_charging_capability.insert(port_id).is_err() {
                error!(
                    "({}): Valid battery charging capability overflow",
                    port.lock().await.name()
                );
            }
        }

        if !port_status.is_connected() {
            // Reset battery charging capability status when disconnected
            let _ = self.ucsi.valid_battery_charging_capability.remove(&port_id);
        }

        if ucsi_event.filter_enabled(self.ucsi.notifications_enabled).is_empty() {
            trace!("{:?}: event received, but no UCSI notifications enabled", port_id);
            return;
        }

        self.pend_ucsi_port(port, port_id).await;
    }

    /// Pend UCSI events for all connected ports
    pub(super) async fn pend_ucsi_connected_ports(&mut self) {
        // Panic Safety: i is limited by the length of port_status
        #[allow(clippy::indexing_slicing)]
        for i in 0..self.registration.ports().len() {
            let port_id = GlobalPortId(i as u8);
            let Some(port) = self.registration.ports().get(i) else {
                error!("Invalid port ID: {}", i);
                continue;
            };

            if let Ok(port_status) = port.lock().await.get_port_status().await {
                if port_status.is_connected() {
                    self.pend_ucsi_port(port, port_id).await;
                }
            } else {
                error!("({}): Failed to get status for port", port.lock().await.name());
            }
        }
    }

    /// Pend a UCSI event for the given port
    async fn pend_ucsi_port(&mut self, port: &'port Reg::Port, port_id: GlobalPortId) {
        if self.ucsi.pending_ports.iter().any(|pending| *pending == port_id) {
            // Already have a pending event for this port, don't need to process it twice
            return;
        }

        // Only notifiy the OPM if we don't have any pending events
        // Once the OPM starts processing events, the next pending port will be sent as part
        // of the CCI response to the ACK_CC_CI command. See [`Self::set_cci_connector_change`]
        let notify_opm = self.ucsi.pending_ports.is_empty();
        if self.ucsi.pending_ports.push_back(port_id).is_ok() {
            self.broadcast_event(Event {
                port,
                event: EventData::UsciChangeIndicator(UsciChangeIndicatorData {
                    port: port_id,
                    notify_opm,
                }),
            })
            .await;
        } else {
            // This shouldn't happen because we have a single slot per port
            // Would likely indicate that an invalid port ID got in somehow
            error!("Pending UCSI events overflow");
        }
    }
}
