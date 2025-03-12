//! Module contain power-policy related message handling
use embedded_services::{
    power::policy::PowerCapability,
    type_c::{
        GlobalPortId, POWER_CAPABILITY_5V_1A5, POWER_CAPABILITY_5V_3A0, POWER_CAPABILITY_USB_DEFAULT_USB2,
        POWER_CAPABILITY_USB_DEFAULT_USB3,
    },
};
use embedded_usb_pd::type_c::Current as TypecCurrent;

use super::*;

/// Threshold power capability before we'll attempt to sink from a dual-role supply
/// This ensures we don't try to sink from something like a phone
const DUAL_ROLE_CONSUMER_THRESHOLD: u32 = 15000;

impl<const N: usize, C: Controller> ControllerWrapper<N, C> {
    /// Return the power device for the given port
    pub(super) fn get_power_device<'a>(
        &'a self,
        port: LocalPortId,
    ) -> Result<&'a policy::device::Device, Error<C::BusError>> {
        if port.0 > N as u8 {
            return PdError::InvalidPort.into();
        }
        Ok(&self.power[port.0 as usize])
    }

    /// Handle a new consumer contract
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    pub(super) async fn process_new_consumer_contract(
        &self,
        controller: &mut C,
        power: &policy::device::Device,
        port: LocalPortId,
        status: &PortStatus,
    ) {
        info!("New consumer contract");

        if let Some(contract) = status.contract {
            if !matches!(contract, Contract::Sink(_)) {
                error!("Not a sink contract");
                return;
            }
        } else {
            error!("No contract");
            return;
        }

        let contract = status.contract.unwrap();
        let capability = PowerCapability::from(contract);
        if status.dual_power && capability.max_power_mw() <= DUAL_ROLE_CONSUMER_THRESHOLD {
            // Don't attempt to sink from a dual-role supply if the power capability is low
            // This is to prevent sinking from a phone or similar device
            // Do a PR swap to become the source instead
            info!(
                "Port{}: Dual-role supply with low power capability, requesting PR swap",
                port.0
            );
            if controller.request_pr_swap(port, PowerRole::Source).await.is_err() {
                error!("Error requesting PR swap");
            }
            return;
        }

        let current_state = power.state().await.kind();
        // Don't update the available consumer contract if we're providing power
        if current_state != StateKind::ConnectedProvider {
            // Recover if we're not in the correct state
            match power.device_action().await {
                action::device::AnyState::Detached(state) => {
                    if let Err(e) = state.attach().await {
                        error!("Error attaching power device: {:?}", e);
                        return;
                    }
                }
                _ => {}
            }

            if let Ok(state) = power.try_device_action::<action::Idle>().await {
                if let Err(e) = state
                    .notify_consumer_power_capability(Some(policy::PowerCapability::from(contract)))
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                    return;
                }
            } else if let Ok(state) = power.try_device_action::<action::ConnectedConsumer>().await {
                if let Err(e) = state
                    .notify_consumer_power_capability(Some(policy::PowerCapability::from(contract)))
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                    return;
                }
            } else {
                error!("Power device not in detached state");
                return;
            }
        }
    }

    /// Handle a new provider contract
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    pub(super) async fn process_new_provider_contract(
        &self,
        port: GlobalPortId,
        power: &policy::device::Device,
        status: &PortStatus,
    ) {
        if port.0 > N as u8 {
            error!("Invalid port {}", port.0);
            return;
        }

        info!("New provider contract");

        if let Some(contract) = status.contract {
            if !matches!(contract, Contract::Source(_)) {
                error!("Not a sink contract");
                return;
            }
        } else {
            error!("No contract");
            return;
        }

        let contract = status.contract.unwrap();
        let current_state = power.state().await.kind();
        // Don't attempt to source if we're consuming power
        if current_state != StateKind::ConnectedConsumer {
            // Recover if we're not in the correct state
            match power.device_action().await {
                action::device::AnyState::Detached(state) => {
                    if let Err(e) = state.attach().await {
                        error!("Error attaching power device: {:?}", e);
                        return;
                    }
                }
                _ => {}
            }

            if let Ok(state) = power.try_device_action::<action::Idle>().await {
                if let Err(e) = state
                    .request_provider_power_capability(policy::PowerCapability::from(contract))
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                }
            } else if let Ok(state) = power.try_device_action::<action::ConnectedProvider>().await {
                if let Err(e) = state
                    .request_provider_power_capability(policy::PowerCapability::from(contract))
                    .await
                {
                    error!("Error setting power contract: {:?}", e);
                }
            } else {
                error!("Power device not in detached state");
                return;
            }
        }
    }

    /// Handle a disconnect command
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    async fn process_disconnect(&self, port: LocalPortId, controller: &mut C, power: &policy::device::Device) {
        let state = power.state().await.kind();

        if state == StateKind::ConnectedConsumer {
            info!("Port{}: Disconnect consumer", port.0);
            if let Err(_) = controller.enable_sink_path(port, false).await {
                error!("Error disabling sink path");
                power.send_response(Err(policy::Error::Failed)).await;
                return;
            }
        } else if state == StateKind::ConnectedProvider {
            info!("Port{}: Disconnect provider", port.0);
            if let Err(_) = controller.enable_source(port, false).await {
                error!("Error disabling source path");
                power.send_response(Err(policy::Error::Failed)).await;
                return;
            }

            // Don't signal since we're disconnected and just resetting to our default value
            if let Err(_) = controller.set_source_current(port, DEFAULT_SOURCE_CURRENT, false).await {
                error!("Error setting source current to default");
                return;
            }
        }
    }

    /// Handle a connect consumer command
    /// None of the event processing functions return errors to allow processing to continue for other ports on a controller
    async fn process_connect_provider(
        &self,
        port: LocalPortId,
        capability: PowerCapability,
        controller: &mut C,
        power: &policy::device::Device,
    ) {
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
                return;
            }
        };

        // Signal since we are supplying a different source current
        if let Err(_) = controller.set_source_current(port, current, true).await {
            error!("Error setting source capability");
            power.send_response(Err(policy::Error::Failed)).await;
            return;
        }
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
                if let Err(_) = controller.enable_sink_path(port, true).await {
                    error!("Error enabling sink path");
                    power.send_response(Err(policy::Error::Failed)).await;
                    return;
                }
            }
            policy::device::RequestData::ConnectProvider(capability) => {
                self.process_connect_provider(port, capability, controller, power).await
            }
            policy::device::RequestData::Disconnect => self.process_disconnect(port, controller, power).await,
        }

        power.send_response(Ok(policy::device::ResponseData::Complete)).await;
    }
}
