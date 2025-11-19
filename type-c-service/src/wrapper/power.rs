//! Module contain power-policy related message handling
use core::future;

use embedded_services::{
    debug,
    ipc::deferred,
    power::policy::{
        ConsumerPowerCapability, ProviderPowerCapability,
        device::{CommandData, InternalResponseData},
    },
};

use crate::wrapper::config::UnconstrainedSink;

use super::*;

impl<'device, M: RawMutex, C: Lockable, V: FwOfferValidator> ControllerWrapper<'device, M, C, V>
where
    <C as Lockable>::Inner: Controller,
{
    /// Return the power device for the given port
    pub fn get_power_device(&self, port: LocalPortId) -> Option<&policy::device::Device> {
        self.registration.power_devices.get(port.0 as usize)
    }

    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(
        &self,
        power: &policy::device::Device,
        status: &PortStatus,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        info!("Process new consumer contract");

        let current_state = power.state().await.kind();
        info!("current power state: {:?}", current_state);

        // Recover if we're not in the correct state
        if status.is_connected() {
            if let action::device::AnyState::Detached(state) = power.device_action().await {
                warn!("Power device is detached, attempting to attach");
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            }
        }

        let available_sink_contract = status.available_sink_contract.map(|c| {
            let mut c: ConsumerPowerCapability = c.into();
            let unconstrained = match self.config.unconstrained_sink {
                UnconstrainedSink::Auto => status.unconstrained_power,
                UnconstrainedSink::PowerThresholdMw(threshold) => c.capability.max_power_mw() >= threshold,
                UnconstrainedSink::Never => false,
            };
            c.flags.set_unconstrained_power(unconstrained);
            c
        });

        if let Ok(state) = power.try_device_action::<action::Idle>().await {
            if let Err(e) = state.notify_consumer_power_capability(available_sink_contract).await {
                error!("Error setting power contract: {:?}", e);
                return PdError::Failed.into();
            }
        } else if let Ok(state) = power.try_device_action::<action::ConnectedConsumer>().await {
            if let Err(e) = state.notify_consumer_power_capability(available_sink_contract).await {
                error!("Error setting power contract: {:?}", e);
                return PdError::Failed.into();
            }
        } else if let Ok(state) = power.try_device_action::<action::ConnectedProvider>().await {
            if let Err(e) = state.notify_consumer_power_capability(available_sink_contract).await {
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
        power: &policy::device::Device,
        status: &PortStatus,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        info!("Process New provider contract");

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
        if status.is_connected() {
            if let action::device::AnyState::Detached(state) = power.device_action().await {
                warn!("Power device is detached, attempting to attach");
                if let Err(e) = state.attach().await {
                    error!("Error attaching power device: {:?}", e);
                    return PdError::Failed.into();
                }
            }
        }

        if let Ok(state) = power.try_device_action::<action::Idle>().await {
            if let Some(contract) = status.available_source_contract {
                if let Err(e) = state.request_provider_power_capability(contract.into()).await {
                    error!("Error setting power contract: {:?}", e);
                    return PdError::Failed.into();
                }
            }
        } else if let Ok(state) = power.try_device_action::<action::ConnectedProvider>().await {
            if let Some(contract) = status.available_source_contract {
                if let Err(e) = state.request_provider_power_capability(contract.into()).await {
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
        controller: &mut C::Inner,
        power: &policy::device::Device,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
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
    fn process_connect_as_provider(
        &self,
        port: LocalPortId,
        capability: ProviderPowerCapability,
        _controller: &mut C::Inner,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        info!("Port{}: Connect as provider: {:#?}", port.0, capability);
        // TODO: double check explicit contract handling
        Ok(())
    }

    /// Wait for a power command
    ///
    /// Returns (local port ID, deferred request)
    /// DROP SAFETY: Call to a select over drop safe futures
    pub(super) async fn wait_power_command(
        &self,
    ) -> (
        LocalPortId,
        deferred::Request<'_, GlobalRawMutex, CommandData, InternalResponseData>,
    ) {
        let futures: [_; MAX_SUPPORTED_PORTS] = from_fn(|i| async move {
            if let Some(device) = self.registration.power_devices.get(i) {
                device.receive().await
            } else {
                future::pending().await
            }
        });
        // DROP SAFETY: Select over drop safe futures
        let (request, local_id) = select_array(futures).await;
        trace!("Power command: device{} {:#?}", local_id, request.command);
        (LocalPortId(local_id as u8), request)
    }

    /// Process a power command
    /// Returns no error because this is a top-level function
    pub(super) async fn process_power_command(
        &self,
        controller: &mut C::Inner,
        state: &mut dyn DynPortState<'_>,
        port: LocalPortId,
        command: &CommandData,
    ) -> InternalResponseData {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        if state.controller_state().fw_update_state.in_progress() {
            debug!("Port{}: Firmware update in progress", port.0);
            return Err(policy::Error::Busy);
        }

        let power = match self.get_power_device(port) {
            Some(power) => power,
            None => {
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
                if self.process_connect_as_provider(port, *capability, controller).is_err() {
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
