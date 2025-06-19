//! Module contain power-policy related message handling
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embedded_services::{
    debug,
    ipc::deferred,
    power::policy::{
        device::{CommandData, InternalResponseData},
        PowerCapability,
    },
};
use embedded_usb_pd::GlobalPortId;

use super::*;

impl<const N: usize, C: Controller, V: FwOfferValidator> ControllerWrapper<'_, N, C, V> {
    /// Return the power device for the given port
    pub(super) fn get_power_device(
        &self,
        port: LocalPortId,
    ) -> Result<&policy::device::Device, Error<<C as Controller>::BusError>> {
        if port.0 > N as u8 {
            return PdError::InvalidPort.into();
        }
        Ok(&self.power[port.0 as usize])
    }

    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(
        &self,
        _controller: &mut C,
        power: &policy::device::Device,
        _port: LocalPortId,
        status: &PortStatus,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        info!("Process new consumer contract");

        let current_state = power.state().await.kind();
        info!("current power state: {:?}", current_state);

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
        } else if let Ok(state) = power.try_device_action::<action::ConnectedProvider>().await {
            if let Err(e) = state
                .notify_consumer_power_capability(status.available_sink_contract)
                .await
            {
                error!("Error setting power contract: {:?}", e);
                return PdError::Failed.into();
            }
        } else {
            error!("Invalid mode");
            return PdError::InvalidMode.into();
        }

        Ok(())
    }

    /// Handle a new contract as provider
    pub(super) async fn process_new_provider_contract(
        &self,
        port: GlobalPortId,
        power: &policy::device::Device,
        status: &PortStatus,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        info!("Process New provider contract");

        if port.0 > N as u8 {
            return PdError::InvalidPort.into();
        }

        let current_state = power.state().await.kind();
        info!("current power state: {:?}", current_state);

        if let action::device::AnyState::ConnectedConsumer(state) = power.device_action().await {
            info!("ConnectedConsumer");
            if let Err(e) = state.detach().await {
                info!("Error detaching power device: {:?}", e);
                return PdError::Failed.into();
            }
        }

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
            error!("Invalid mode");
            return PdError::InvalidMode.into();
        }

        Ok(())
    }

    /// Handle a disconnect command
    async fn process_disconnect(
        &self,
        port: LocalPortId,
        controller: &mut C,
        power: &policy::device::Device,
    ) -> Result<(), Error<<C as Controller>::BusError>> {
        let state = power.state().await.kind();
        if state == StateKind::ConnectedConsumer {
            info!("Port{}: Disconnect from ConnectedConsumer", port.0);
            if controller.enable_sink_path(port, false).await.is_err() {
                error!("Error disabling sink path");
                return PdError::Failed.into();
            }
        }

        Ok(())
    }

    /// Handle a connect as provider command
    async fn process_connect_as_provider(
        &self,
        port: LocalPortId,
        capability: PowerCapability,
        _controller: &mut C,
    ) -> Result<(), Error<C::BusError>> {
        info!("Port{}: Connect as provider: {:#?}", port.0, capability);
        // TODO: double check explicit contract handling
        Ok(())
    }

    /// Wait for a power command
    pub(super) async fn wait_power_command(
        &self,
    ) -> (
        deferred::Request<'_, NoopRawMutex, CommandData, InternalResponseData>,
        LocalPortId,
    ) {
        let futures: [_; N] = from_fn(|i| self.power[i].receive());
        let (request, local_id) = select_array(futures).await;
        trace!("Power command: device{} {:#?}", local_id, request.command);
        (request, LocalPortId(local_id as u8))
    }

    /// Process a power command
    /// Returns no error because this is a top-level function
    pub(super) async fn process_power_command(
        &self,
        controller: &mut C,
        state: &mut InternalState,
        port: LocalPortId,
        command: &CommandData,
    ) -> InternalResponseData {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        if state.fw_update_state.in_progress() {
            debug!("Port{}: Firmware update in progress", port.0);
            return Err(policy::Error::Busy);
        }

        let power = match self.get_power_device(port) {
            Ok(power) => power,
            Err(_) => {
                error!("Port{}: Error getting power device for port", port.0);
                return Err(policy::Error::InvalidDevice);
            }
        };

        match command {
            policy::device::CommandData::ConnectAsConsumer(capability) => {
                info!(
                    "Port{}: Connect as consumer: {:?}, enable input switch",
                    port.0, capability
                );
                if controller.enable_sink_path(port, true).await.is_err() {
                    error!("Error enabling sink path");
                    return Err(policy::Error::Failed);
                }
            }
            policy::device::CommandData::ConnectAsProvider(capability) => {
                if self
                    .process_connect_as_provider(port, *capability, controller)
                    .await
                    .is_err()
                {
                    error!("Error processing connect provider");
                    return Err(policy::Error::Failed);
                }
            }
            policy::device::CommandData::Disconnect => {
                if self.process_disconnect(port, controller, power).await.is_err() {
                    error!("Error processing disconnect");
                    return Err(policy::Error::Failed);
                }
            }
        }

        Ok(policy::device::ResponseData::Complete)
    }
}
