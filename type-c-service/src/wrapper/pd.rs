use crate::wrapper::event_receiver::SinkReadyTimeoutEvent;
use embassy_time::Duration;
use embedded_services::debug;
use embedded_usb_pd::constants::{T_PS_TRANSITION_EPR_MS, T_PS_TRANSITION_SPR_MS};
use embedded_usb_pd::ucsi::{self, lpm};
use power_policy_interface::psu::{self, PsuState};
use type_c_interface::port;
use type_c_interface::port::{InternalResponseData, Response};

use super::*;

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, V>
where
    D::Inner: Controller,
{
    /// Check the sink ready timeout
    ///
    /// After accepting a sink contract (new contract as consumer), the PD spec guarantees that the
    /// source will be available to provide power after `tPSTransition`. This allows us to handle transitions
    /// even for controllers that might not always broadcast sink ready events.
    pub(super) fn check_sink_ready_timeout<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        previous_status: &PortStatus,
        new_status: &PortStatus,
        port: LocalPortId,
        new_contract: bool,
        sink_ready: bool,
    ) -> Result<(), PdError> {
        let contract_changed = previous_status.available_sink_contract != new_status.available_sink_contract;
        let timeout = sink_ready_timeout.get_timeout(port);

        // Don't start the timeout if the sink has signaled it's ready or if the contract didn't change.
        // The latter ensures that soft resets won't continually reset the ready timeout
        debug!(
            "Port{}: Check sink ready: new_contract={:?}, sink_ready={:?}, contract_changed={:?}, deadline={:?}",
            port.0, new_contract, sink_ready, contract_changed, timeout,
        );
        if new_contract && !sink_ready && contract_changed {
            // Start the timeout
            // Double the spec maximum transition time to provide a safety margin for hardware/controller delays or out-of-spec controllers.
            let timeout_ms = if new_status.epr {
                T_PS_TRANSITION_EPR_MS
            } else {
                T_PS_TRANSITION_SPR_MS
            }
            .maximum
            .0 * 2;

            debug!("Port{}: Sink ready timeout started for {}ms", port.0, timeout_ms);
            sink_ready_timeout.set_timeout(port, Instant::now() + Duration::from_millis(timeout_ms as u64));
        } else if timeout.is_some()
            && (!new_status.is_connected() || new_status.available_sink_contract.is_none() || sink_ready)
        {
            debug!("Port{}: Sink ready timeout cleared", port.0);
            sink_ready_timeout.clear_timeout(port);
        }
        Ok(())
    }

    /// Process a request to set the maximum sink voltage for a port
    async fn process_set_max_sink_voltage(
        &self,
        controller: &mut D::Inner,
        port_state: &mut PortState<S>,
        state: &mut psu::State,
        local_port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> Result<port::PortResponseData, PdError> {
        let psu_state = state.psu_state;
        debug!("Port{}: Current state: {:#?}", local_port.0, psu_state);
        if matches!(psu_state, PsuState::ConnectedConsumer(_)) {
            debug!("Port{}: Set max sink voltage, connected consumer found", local_port.0);
            if voltage_mv.is_some() && voltage_mv < state.consumer_capability.map(|c| c.capability.voltage_mv) {
                // New max voltage is lower than current consumer capability which will trigger a renegociation
                // So disconnect first
                debug!(
                    "Port{}: Disconnecting consumer before setting max sink voltage",
                    local_port.0
                );
                let _ = state.disconnect(true);
                port_state
                    .power_policy_sender
                    .send(power_policy_interface::psu::event::EventData::Disconnected)
                    .await;
            }
        }

        match controller.set_max_sink_voltage(local_port, voltage_mv).await {
            Ok(()) => Ok(port::PortResponseData::Complete),
            Err(e) => match e {
                Error::Bus(_) => Err(PdError::Failed),
                Error::Pd(e) => Err(e),
            },
        }
    }

    /// Handle a port command
    async fn process_port_command(
        &self,
        cfu_event_receiver: &mut CfuEventReceiver,
        controller: &mut D::Inner,
        command: &port::PortCommand,
    ) -> Response<'static> {
        if cfu_event_receiver.fw_update_state.in_progress() {
            debug!("FW update in progress, ignoring port command");
            return port::Response::Port(Err(PdError::Busy));
        }

        let local_port = if let Ok(port) = self.registration.pd_controller.lookup_local_port(command.port) {
            port
        } else {
            debug!("Invalid port: {:?}", command.port);
            return port::Response::Port(Err(PdError::InvalidPort));
        };

        let Some(port) = self.ports.get(local_port.0 as usize) else {
            debug!("Invalid port: {:?}", command.port);
            return port::Response::Port(Err(PdError::InvalidPort));
        };

        let mut port_state = port.state.lock().await;
        port::Response::Port(match command.data {
            port::PortCommandData::RetimerFwUpdateGetState => {
                match controller.get_rt_fw_update_status(local_port).await {
                    Ok(status) => Ok(port::PortResponseData::RtFwUpdateStatus(status)),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::RetimerFwUpdateSetState => {
                match controller.set_rt_fw_update_state(local_port).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::RetimerFwUpdateClearState => {
                match controller.clear_rt_fw_update_state(local_port).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::SetRetimerCompliance => match controller.set_rt_compliance(local_port).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::ReconfigureRetimer => match controller.reconfigure_retimer(local_port).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SetMaxSinkVoltage(voltage_mv) => {
                match self.registration.pd_controller.lookup_local_port(command.port) {
                    Ok(local_port) => {
                        let mut proxy = port.proxy.lock().await;
                        self.process_set_max_sink_voltage(
                            controller,
                            &mut port_state,
                            &mut proxy.psu_state,
                            local_port,
                            voltage_mv,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                }
            }
            port::PortCommandData::SetUnconstrainedPower(unconstrained) => {
                match controller.set_unconstrained_power(local_port, unconstrained).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::ClearDeadBatteryFlag => match controller.clear_dead_battery_flag(local_port).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::GetOtherVdm => match controller.get_other_vdm(local_port).await {
                Ok(vdm) => {
                    debug!("Port{}: Other VDM: {:?}", local_port.0, vdm);
                    Ok(port::PortResponseData::OtherVdm(vdm))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::GetAttnVdm => match controller.get_attn_vdm(local_port).await {
                Ok(vdm) => {
                    debug!("Port{}: Attention VDM: {:?}", local_port.0, vdm);
                    Ok(port::PortResponseData::AttnVdm(vdm))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SendVdm(tx_vdm) => match controller.send_vdm(local_port, tx_vdm).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SetUsbControl(config) => {
                match controller.set_usb_control(local_port, config).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::GetDpStatus => match controller.get_dp_status(local_port).await {
                Ok(status) => {
                    debug!("Port{}: DP Status: {:?}", local_port.0, status);
                    Ok(port::PortResponseData::DpStatus(status))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SetDpConfig(config) => match controller.set_dp_config(local_port, config).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::ExecuteDrst => match controller.execute_drst(local_port).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SetTbtConfig(config) => match controller.set_tbt_config(local_port, config).await {
                Ok(()) => Ok(port::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            port::PortCommandData::SetPdStateMachineConfig(config) => {
                match controller.set_pd_state_machine_config(local_port, config).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::SetTypeCStateMachineConfig(state) => {
                match controller.set_type_c_state_machine_config(local_port, state).await {
                    Ok(()) => Ok(port::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            port::PortCommandData::ExecuteUcsiCommand(command_data) => Ok(port::PortResponseData::UcsiResponse(
                controller
                    .execute_ucsi_command(lpm::Command::new(local_port, command_data))
                    .await
                    .map_err(|e| match e {
                        Error::Bus(_) => PdError::Failed,
                        Error::Pd(e) => e,
                    }),
            )),
        })
    }

    async fn process_controller_command(
        &self,
        cfu_event_receiver: &mut CfuEventReceiver,
        controller: &mut D::Inner,
        command: &port::InternalCommandData,
    ) -> Response<'static> {
        if cfu_event_receiver.fw_update_state.in_progress() {
            debug!("FW update in progress, ignoring controller command");
            return port::Response::Controller(Err(PdError::Busy));
        }

        match command {
            port::InternalCommandData::Status => {
                let status = controller.get_controller_status().await;
                port::Response::Controller(status.map(InternalResponseData::Status).map_err(|_| PdError::Failed))
            }
            port::InternalCommandData::SyncState => {
                let result = self.sync_state_internal(controller).await;
                port::Response::Controller(
                    result
                        .map(|_| InternalResponseData::Complete)
                        .map_err(|_| PdError::Failed),
                )
            }
            port::InternalCommandData::Reset => {
                let result = controller.reset_controller().await;
                port::Response::Controller(
                    result
                        .map(|_| InternalResponseData::Complete)
                        .map_err(|_| PdError::Failed),
                )
            }
        }
    }

    /// Handle a PD controller command
    pub(super) async fn process_pd_command(
        &self,
        cfu_event_receiver: &mut CfuEventReceiver,
        controller: &mut D::Inner,
        command: &port::Command,
    ) -> Response<'static> {
        match command {
            port::Command::Port(command) => self.process_port_command(cfu_event_receiver, controller, command).await,
            port::Command::Controller(command) => {
                self.process_controller_command(cfu_event_receiver, controller, command)
                    .await
            }
            port::Command::Lpm(_) => port::Response::Ucsi(ucsi::Response {
                cci: ucsi::cci::Cci::new_error(),
                data: None,
            }),
        }
    }
}
