//! Temporary bridge between a controller and the type-C service

use embedded_services::{debug, sync::Lockable};
use embedded_usb_pd::{Error, PdError, ucsi::lpm};
use type_c_interface::port::{self, Controller as _, InternalResponseData, Response};

use crate::bridge::event_receiver::{ControllerCommand, OutputControllerCommand};
pub mod event_receiver;

pub struct Bridge<'device, Controller: Lockable<Inner: port::Controller>> {
    controller: &'device Controller,
    registration: &'static port::Device<'static>,
}

impl<'device, Controller: Lockable<Inner: port::Controller>> Bridge<'device, Controller> {
    pub fn new(controller: &'device Controller, registration: &'static port::Device<'static>) -> Self {
        Self {
            controller,
            registration,
        }
    }

    /// Handle a port command
    pub async fn process_port_command(&mut self, command: &port::PortCommand) -> Response<'static> {
        let local_port = if let Ok(port) = self.registration.lookup_local_port(command.port) {
            port
        } else {
            debug!("Invalid port: {:?}", command.port);
            return port::Response::Port(Err(PdError::InvalidPort));
        };

        let mut controller = self.controller.lock().await;
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
            // This command isn't sent by the type-C service, disable it for the transition
            port::PortCommandData::SetMaxSinkVoltage(_) => Ok(port::PortResponseData::Complete),
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

    pub async fn process_controller_command(&mut self, command: &port::InternalCommandData) -> Response<'static> {
        let mut controller = self.controller.lock().await;
        match command {
            port::InternalCommandData::Status => {
                let status = controller.get_controller_status().await;
                port::Response::Controller(status.map(InternalResponseData::Status).map_err(|_| PdError::Failed))
            }
            // This isn't sent by the type-C service, disable it for the transition
            port::InternalCommandData::SyncState => port::Response::Controller(Ok(InternalResponseData::Complete)),
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
    pub async fn process_event(&mut self, command: ControllerCommand<'static>) -> OutputControllerCommand<'static> {
        let response = match command.command {
            port::Command::Port(command) => self.process_port_command(&command).await,
            port::Command::Controller(command) => self.process_controller_command(&command).await,
        };
        OutputControllerCommand {
            request: command,
            response,
        }
    }
}
