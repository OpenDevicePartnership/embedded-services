use embassy_futures::yield_now;
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Timer};
use embedded_services::debug;
use embedded_services::type_c::controller::{InternalResponseData, Response};
use embedded_services::type_c::Cached;
use embedded_usb_pd::constants::{T_SRC_TRANS_REQ_EPR_MS, T_SRC_TRANS_REQ_SPR_MS};
use embedded_usb_pd::ucsi;

use super::*;

impl<'a, const N: usize, C: Controller, BACK: Backing<'a>, V: FwOfferValidator> ControllerWrapper<'a, N, C, BACK, V> {
    async fn process_get_pd_alert(&self, local_port: LocalPortId) -> Result<Option<Ado>, PdError> {
        let mut backing = self.backing.lock().await;
        let mut channel = match backing.pd_alert_channel_mut(local_port.0 as usize).await {
            Some(channel) => channel,
            None => return Err(PdError::InvalidPort),
        };

        loop {
            match channel.1.try_next_message() {
                Some(WaitResult::Message(alert)) => return Ok(Some(alert)),
                None => return Ok(None),
                Some(WaitResult::Lagged(count)) => {
                    warn!("Port{}: Lagged PD alert channel: {}", local_port.0, count);
                    // Yield to avoid starving other tasks since we're in a loop and try_next_message isn't async
                    yield_now().await;
                }
            }
        }
    }

    /// Check the sink ready timeout
    ///
    /// After accepting a sink contract (new contract as consumer), the PD spec guarantees that the
    /// source will be available to provide power after `tSrcTransReq`. This allows us to handle transitions
    /// even for controllers that might not always broadcast sink ready events.
    pub(super) async fn check_sink_ready_timeout(
        &self,
        state: &mut InternalState<N>,
        status: &PortStatus,
        port: LocalPortId,
        new_contract: bool,
        sink_ready: bool,
    ) -> Result<(), PdError> {
        if port.0 as usize >= N {
            return Err(PdError::InvalidPort);
        }

        let deadline = &mut state.port_states[port.0 as usize].sink_ready_deadline;

        if new_contract && !sink_ready {
            // Start the timeout
            let timeout_ms = if status.epr {
                T_SRC_TRANS_REQ_EPR_MS
            } else {
                T_SRC_TRANS_REQ_SPR_MS
            };
            debug!("Port{}: Sink ready timeout started for {}ms", port.0, timeout_ms);
            *deadline = Some(Instant::now() + Duration::from_millis(timeout_ms as u64));
        } else if deadline.is_some()
            && (!status.is_connected() || status.available_sink_contract.is_none() || sink_ready)
        {
            // Clear the timeout
            debug!("Port{}: Sink ready timeout cleared", port.0);
            *deadline = None;
        }
        Ok(())
    }

    /// Wait for a sink ready timeout and return the port that has timed out.
    ///
    /// DROP SAFETY: No state to restore
    pub(super) async fn wait_sink_ready_timeout(&self) -> LocalPortId {
        let futures: [_; N] = from_fn(|i| async move {
            let deadline = self.state.lock().await.port_states[i].sink_ready_deadline;
            if let Some(deadline) = deadline {
                Timer::at(deadline).await;
                debug!("Port{}: Sink ready timeout reached", i);
                self.state.lock().await.port_states[i].sink_ready_deadline = None;
            } else {
                pending::<()>().await;
            }
        });

        // DROP SAFETY: Select over drop safe futures
        let (_, port_index) = select_array(futures).await;
        LocalPortId(port_index as u8)
    }

    /// Set the maximum sink voltage for a port
    pub async fn set_max_sink_voltage(&self, local_port: LocalPortId, voltage_mv: Option<u16>) -> Result<(), PdError> {
        let mut controller = self.get_inner_mut().await;
        let _ = self
            .process_set_max_sink_voltage(&mut controller, local_port, voltage_mv)
            .await?;
        Ok(())
    }

    /// Process a request to set the maximum sink voltage for a port
    async fn process_set_max_sink_voltage(
        &self,
        controller: &mut C,
        local_port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> Result<controller::PortResponseData, PdError> {
        let power_device = self.get_power_device(local_port)?;

        let state = power_device.state().await;
        debug!("Port{}: Current state: {:#?}", local_port.0, state);
        if let Ok(connected_consumer) = power_device.try_device_action::<action::ConnectedConsumer>().await {
            debug!("Port{}: Set max sink voltage, connected consumer found", local_port.0);
            if voltage_mv.is_some()
                && voltage_mv
                    < power_device
                        .consumer_capability()
                        .await
                        .map(|c| c.capability.voltage_mv)
            {
                // New max voltage is lower than current consumer capability which will trigger a renegociation
                // So disconnect first
                debug!(
                    "Port{}: Disconnecting consumer before setting max sink voltage",
                    local_port.0
                );
                let _ = connected_consumer.disconnect().await;
            }
        }

        match controller.set_max_sink_voltage(local_port, voltage_mv).await {
            Ok(()) => Ok(controller::PortResponseData::Complete),
            Err(e) => match e {
                Error::Bus(_) => Err(PdError::Failed),
                Error::Pd(e) => Err(e),
            },
        }
    }

    async fn process_get_port_status(
        &self,
        controller: &mut C,
        state: &mut InternalState<N>,
        local_port: LocalPortId,
        cached: Cached,
    ) -> Result<controller::PortResponseData, PdError> {
        if cached.0 {
            Ok(controller::PortResponseData::PortStatus(
                state.port_states[local_port.0 as usize].status,
            ))
        } else {
            match controller.get_port_status(local_port).await {
                Ok(status) => Ok(controller::PortResponseData::PortStatus(status)),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            }
        }
    }

    /// Handle a port command
    async fn process_port_command(
        &self,
        controller: &mut C,
        state: &mut InternalState<N>,
        command: &controller::PortCommand,
    ) -> Response<'static> {
        if state.fw_update_state.in_progress() {
            debug!("FW update in progress, ignoring port command");
            return controller::Response::Port(Err(PdError::Busy));
        }

        let local_port = self.pd_controller.lookup_local_port(command.port);
        if local_port.is_err() {
            return controller::Response::Port(Err(PdError::InvalidPort));
        }

        let local_port = local_port.unwrap();
        controller::Response::Port(match command.data {
            controller::PortCommandData::PortStatus(cached) => {
                self.process_get_port_status(controller, state, local_port, cached)
                    .await
            }
            controller::PortCommandData::ClearEvents => {
                let port_index = local_port.0 as usize;
                let event = core::mem::take(&mut state.port_states[port_index].pending_events);
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
            controller::PortCommandData::SetRetimerCompliance => match controller.set_rt_compliance(local_port).await {
                Ok(()) => Ok(controller::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::ReconfigureRetimer => match controller.reconfigure_retimer(local_port).await {
                Ok(()) => Ok(controller::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::GetPdAlert => match self.process_get_pd_alert(local_port).await {
                Ok(alert) => Ok(controller::PortResponseData::PdAlert(alert)),
                Err(e) => Err(e),
            },
            controller::PortCommandData::SetMaxSinkVoltage(voltage_mv) => {
                match self.pd_controller.lookup_local_port(command.port) {
                    Ok(local_port) => {
                        self.process_set_max_sink_voltage(controller, local_port, voltage_mv)
                            .await
                    }
                    Err(e) => Err(e),
                }
            }
            controller::PortCommandData::SetUnconstrainedPower(unconstrained) => {
                match controller.set_unconstrained_power(local_port, unconstrained).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            controller::PortCommandData::ClearDeadBatteryFlag => {
                match controller.clear_dead_battery_flag(local_port).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            controller::PortCommandData::GetOtherVdm => match controller.get_other_vdm(local_port).await {
                Ok(vdm) => {
                    debug!("Port{}: Other VDM: {:?}", local_port.0, vdm);
                    Ok(controller::PortResponseData::OtherVdm(vdm))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::GetAttnVdm => match controller.get_attn_vdm(local_port).await {
                Ok(vdm) => {
                    debug!("Port{}: Attention VDM: {:?}", local_port.0, vdm);
                    Ok(controller::PortResponseData::AttnVdm(vdm))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::SendVdm(tx_vdm) => match controller.send_vdm(local_port, tx_vdm).await {
                Ok(()) => Ok(controller::PortResponseData::Complete),
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::SetUsbControl(config) => {
                match controller.set_usb_control(local_port, config).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
            controller::PortCommandData::GetDpStatus => match controller.get_dp_status(local_port).await {
                Ok(status) => {
                    debug!("Port{}: DP Status: {:?}", local_port.0, status);
                    Ok(controller::PortResponseData::DpStatus(status))
                }
                Err(e) => match e {
                    Error::Bus(_) => Err(PdError::Failed),
                    Error::Pd(e) => Err(e),
                },
            },
            controller::PortCommandData::SetDpConfig(config) => {
                match controller.set_dp_config(local_port, config).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            },
            controller::PortCommandData::ExecuteDrst => {
                match controller.execute_drst(local_port).await {
                    Ok(()) => Ok(controller::PortResponseData::Complete),
                    Err(e) => match e {
                        Error::Bus(_) => Err(PdError::Failed),
                        Error::Pd(e) => Err(e),
                    },
                }
            }
        })
    }

    async fn process_controller_command(
        &self,
        controller: &mut C,
        state: &mut InternalState<N>,
        command: &controller::InternalCommandData,
    ) -> Response<'static> {
        if state.fw_update_state.in_progress() {
            debug!("FW update in progress, ignoring controller command");
            return controller::Response::Controller(Err(PdError::Busy));
        }

        match command {
            controller::InternalCommandData::Status => {
                let status = controller.get_controller_status().await;
                controller::Response::Controller(status.map(InternalResponseData::Status).map_err(|_| PdError::Failed))
            }
            controller::InternalCommandData::SyncState => {
                let result = self.sync_state_internal(controller, state).await;
                controller::Response::Controller(
                    result
                        .map(|_| InternalResponseData::Complete)
                        .map_err(|_| PdError::Failed),
                )
            }
            controller::InternalCommandData::Reset => {
                let result = controller.reset_controller().await;
                controller::Response::Controller(
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
        controller: &mut C,
        state: &mut InternalState<N>,
        command: &controller::Command,
    ) -> Response<'static> {
        match command {
            controller::Command::Port(command) => self.process_port_command(controller, state, command).await,
            controller::Command::Controller(command) => {
                self.process_controller_command(controller, state, command).await
            }
            controller::Command::Lpm(_) => controller::Response::Ucsi(ucsi::Response {
                cci: ucsi::cci::Cci::new_error(),
                data: None,
            }),
        }
    }
}
