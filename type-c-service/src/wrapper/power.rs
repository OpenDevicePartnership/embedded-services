//! Module contain power-policy related message handling
use embedded_services::{
    power::policy::{device::RequestData, PowerCapability},
    type_c::{
        POWER_CAPABILITY_5V_1A5, POWER_CAPABILITY_5V_3A0, POWER_CAPABILITY_USB_DEFAULT_USB2,
        POWER_CAPABILITY_USB_DEFAULT_USB3,
    },
};
use embedded_usb_pd::{GlobalPortId, PowerRole};

use super::*;

impl<const N: usize, C: Controller> ControllerWrapper<'_, N, C> {
    /// Return the power device for the given port
    pub(super) fn get_power_device(&self, port: LocalPortId) -> Result<&policy::device::Device, Error<C::BusError>> {
        if port.0 > N as u8 {
            return PdError::InvalidPort.into();
        }
        Ok(&self.power[port.0 as usize])
    }

    /// Handle a new consumer contract
    pub(super) async fn process_new_consumer_contract(
        &self,
        controller: &mut C,
        power: &policy::device::Device,
        port: LocalPortId,
        status: &PortStatus,
    ) -> Result<(), Error<C::BusError>> {
        info!("New consumer contract");

        if let Some(capability) = status.available_sink_contract {
            if status.dual_power && capability.max_power_mw() <= DUAL_ROLE_CONSUMER_THRESHOLD_MW {
                // Don't attempt to sink from a dual-role supply if the power capability is low
                // This is to prevent sinking from a phone or similar device
                // Do a PR swap to become the source instead
                info!(
                    "Port{}: Dual-role supply with low power capability, requesting PR swap",
                    port.0
                );
                if controller.request_pr_swap(port, PowerRole::Source).await.is_err() {
                    error!("Error requesting PR swap");
                    return PdError::Failed.into();
                }
                return Ok(());
            }
        }

        let current_state = power.state().await.kind();
        // Don't update the available consumer contract if we're providing power
        if current_state != StateKind::ConnectedProvider {
            // Recover if we're not in the correct state
            if let action::device::AnyState::Detached(state) = power.device_action().await {
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            }

            if let Ok(state) = power.try_device_action::<action::Idle>().await {
                if let Err(e) = state
                    .notify_consumer_power_capability(status.available_sink_contract)
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                    return PdError::Failed.into();
                }
            } else if let Ok(state) = power.try_device_action::<action::ConnectedConsumer>().await {
                if let Err(e) = state
                    .notify_consumer_power_capability(status.available_sink_contract)
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                    return PdError::Failed.into();
                }
            } else {
                error!("Power device not in detached state");
                return PdError::InvalidMode.into();
            }
        }

        Ok(())
    }

    /// Handle a new provider contract
    pub(super) async fn process_new_provider_contract(
        &self,
        port: GlobalPortId,
        power: &policy::device::Device,
        status: &PortStatus,
    ) -> Result<(), Error<C::BusError>> {
        if port.0 > N as u8 {
            return PdError::InvalidPort.into();
        }

        let current_state = power.state().await.kind();
        // Don't attempt to source if we're consuming power
        if current_state != StateKind::ConnectedConsumer {
            // Recover if we're not in the correct state
            if let action::device::AnyState::Detached(state) = power.device_action().await {
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            }

            if let Ok(state) = power.try_device_action::<action::Idle>().await {
                if let Some(contract) = status.available_source_contract {
                    if let Err(e) = state.request_provider_power_capability(contract).await {
                        error!("Error setting power contract: {:?}", e);
                        return PdError::Failed.into();
                    }
                }
            } else if let Ok(state) = power.try_device_action::<action::ConnectedProvider>().await {
                if let Some(contract) = status.available_source_contract {
                    if let Err(e) = state.request_provider_power_capability(contract).await {
                        error!("Error setting power contract: {:?}", e);
                        return PdError::Failed.into();
                    }
                } else {
                    // No longer need to source, so disconnect
                    if let Err(e) = state.disconnect().await {
                        error!("Error setting power contract: {:?}", e);
                        return PdError::Failed.into();
                    }
                }
            } else {
                error!("Power device not in detached state");
                return PdError::InvalidMode.into();
            }
        }

        Ok(())
    }

    /// Handle a disconnect command
    async fn process_disconnect(
        &self,
        port: LocalPortId,
        controller: &mut C,
        power: &policy::device::Device,
    ) -> Result<(), Error<C::BusError>> {
        let state = power.state().await.kind();

        if state == StateKind::ConnectedConsumer {
            info!("Port{}: Disconnect consumer", port.0);
            if controller.enable_sink_path(port, false).await.is_err() {
                error!("Error disabling sink path");
                power.send_response(Err(policy::Error::Failed)).await;
                return PdError::Failed.into();
            }
        } else if state == StateKind::ConnectedProvider {
            info!("Port{}: Disconnect provider", port.0);
            if controller.set_sourcing(port, false).await.is_err() {
                error!("Error disabling source path");
                power.send_response(Err(policy::Error::Failed)).await;
                return PdError::Failed.into();
            }

            // Don't signal since we're disconnected and just resetting to our default value
            if controller
                .set_source_current(port, DEFAULT_SOURCE_CURRENT, false)
                .await
                .is_err()
            {
                error!("Error setting source current to default");
                return PdError::Failed.into();
            }
        }

        Ok(())
    }

    /// Handle a connect consumer command
    async fn process_connect_provider(
        &self,
        port: LocalPortId,
        capability: PowerCapability,
        controller: &mut C,
        power: &policy::device::Device,
    ) -> Result<(), Error<C::BusError>> {
        info!("Port{}: Connect provider: {:#?}", port.0, capability);
        let current = match capability {
            POWER_CAPABILITY_USB_DEFAULT_USB2 | POWER_CAPABILITY_USB_DEFAULT_USB3 => TypecCurrent::UsbDefault,
            POWER_CAPABILITY_5V_1A5 => TypecCurrent::Current1A5,
            POWER_CAPABILITY_5V_3A0 => TypecCurrent::Current3A0,
            _ => {
                error!("Invalid power capability");
                power
                    .send_response(Err(policy::Error::CannotProvide(Some(capability))))
                    .await;
                return PdError::InvalidParams.into();
            }
        };

        // Signal since we are supplying a different source current
        if controller.set_source_current(port, current, true).await.is_err() {
            error!("Error setting source capability");
            power.send_response(Err(policy::Error::Failed)).await;
            return PdError::Failed.into();
        }

        Ok(())
    }

    /// Wait for a power command
    pub(super) async fn wait_power_command(&self) -> (RequestData, LocalPortId) {
        let futures: [_; N] = from_fn(|i| self.power[i].wait_request());

        let (command, local_id) = select_array(futures).await;
        trace!("Power command: device{} {:#?}", local_id, command);
        (command, LocalPortId(local_id as u8))
    }

    /// Process a power command
    /// Returns no error because this is a top-level function
    pub(super) async fn process_power_command(&self, controller: &mut C, port: LocalPortId, command: RequestData) {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        let power = match self.get_power_device(port) {
            Ok(power) => power,
            Err(_) => {
                error!("Port{}: Error getting power device for port", port.0);
                return;
            }
        };

        match command {
            policy::device::RequestData::ConnectConsumer(capability) => {
                info!("Port{}: Connect consumer: {:?}", port.0, capability);
                if controller.enable_sink_path(port, true).await.is_err() {
                    error!("Error enabling sink path");
                    power.send_response(Err(policy::Error::Failed)).await;
                    return;
                }
            }
            policy::device::RequestData::ConnectProvider(capability) => {
                if self
                    .process_connect_provider(port, capability, controller, power)
                    .await
                    .is_err()
                {
                    error!("Error processing connect provider");
                    return;
                }
            }
            policy::device::RequestData::Disconnect => {
                if self.process_disconnect(port, controller, power).await.is_err() {
                    error!("Error processing disconnect");
                    return;
                }
            }
        }

        power.send_response(Ok(policy::device::ResponseData::Complete)).await;
    }
}
