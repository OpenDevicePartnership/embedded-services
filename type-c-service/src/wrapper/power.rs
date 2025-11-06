//! Module contain power-policy related message handling
use core::pin::pin;

use embassy_futures::select::select_slice;
use embedded_services::{
    debug,
    power::policy::{
        ConsumerPowerCapability, ProviderPowerCapability,
        device::{CommandData, InternalResponseData, ResponseData},
    },
};

use embedded_services::power::policy::Error as PowerError;
use embedded_services::power::policy::device::CommandData as PowerCommand;

use super::*;

impl<
    'device,
    M: RawMutex,
    C: Lockable,
    S: event::Sender<policy::RequestData>,
    R: event::Receiver<policy::RequestData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, C, S, R, V>
where
    <C as Lockable>::Inner: Controller,
{
    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(
        &self,
        power: &mut PortPower<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        info!("Process new consumer contract");

        let current_state = power.state.state();
        info!("current power state: {:?}", current_state);

        let available_sink_contract = status.available_sink_contract.map(|c| {
            let mut c: ConsumerPowerCapability = c.into();
            c.flags.set_unconstrained_power(status.unconstrained_power);
            c
        });

        if let Err(e) = power.state.update_consumer_power_capability(available_sink_contract) {
            warn!(
                "Device was not in correct state for consumer contract, recovered: {:#?}",
                e
            );
        }
        Ok(())
    }

    /// Handle a new contract as provider
    pub(super) async fn process_new_provider_contract(
        &self,
        power: &mut PortPower<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        info!("Process New provider contract");

        let current_state = power.state.state();
        info!("current power state: {:?}", current_state);

        if let Err(e) = power.state.update_requested_provider_power_capability(
            status.available_sink_contract.map(ProviderPowerCapability::from),
        ) {
            warn!(
                "Device was not in correct state for provider contract, recovered: {:#?}",
                e
            );
        }
        Ok(())
    }

    /// Handle a disconnect command
    async fn process_disconnect(
        &self,
        port: LocalPortId,
        controller: &mut C::Inner,
        power: &mut PortPower<S>,
    ) -> Result<(), Error<<C::Inner as Controller>::BusError>> {
        if power.state.state().kind() == StateKind::ConnectedConsumer {
            info!("Port{}: Disconnect from ConnectedConsumer", port.0);
            if controller.enable_sink_path(port, false).await.is_err() {
                error!("Error disabling sink path");
                return PdError::Failed.into();
            }

            if let Err(e) = power.state.disconnect(false) {
                warn!(
                    "{:?}: Device was not in correct state for disconnect, recovered: {:#?}",
                    port, e
                );
            }
        }

        Ok(())
    }

    /// Handle a connect as provider command
    async fn process_connect_as_provider(
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
    pub(super) async fn wait_power_command(&self) -> (LocalPortId, CommandData) {
        let mut futures = heapless::Vec::<_, MAX_SUPPORTED_PORTS>::new();
        for receiver in self.power_proxy_receivers {
            if futures
                .push(async {
                    let mut lock = receiver.lock().await;
                    lock.receive().await
                })
                .is_err()
            {
                error!("Futures vec overflow");
            }
        }

        // DROP SAFETY: Select over drop safe futures
        let (request, local_id) = select_slice(pin!(futures.as_mut_slice())).await;
        trace!("Power command: device{} {:#?}", local_id, request);
        (LocalPortId(local_id as u8), request)
    }

    /// Process a power command
    /// Returns no error because this is a top-level function
    pub(super) async fn process_power_command(
        &self,
        controller: &mut C::Inner,
        state: &mut dyn DynPortState<'_, S>,
        port: LocalPortId,
        command: &CommandData,
    ) -> InternalResponseData {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        if state.controller_state().fw_update_state.in_progress() {
            debug!("Port{}: Firmware update in progress", port.0);
            return Err(PowerError::Busy);
        }

        let power = state.port_power_mut().get_mut(port.0 as usize);
        if power.is_none() {
            return Err(PowerError::InvalidDevice);
        }

        let power = power.unwrap();
        match command {
            PowerCommand::ConnectAsConsumer(capability) => {
                info!(
                    "Port{}: Connect as consumer: {:?}, enable input switch",
                    port.0, capability
                );
                if controller.enable_sink_path(port, true).await.is_err() {
                    error!("Error enabling sink path");
                    return Err(PowerError::Failed);
                }
            }
            PowerCommand::ConnectAsProvider(capability) => {
                if self
                    .process_connect_as_provider(port, *capability, controller)
                    .await
                    .is_err()
                {
                    error!("Error processing connect provider");
                    return Err(PowerError::Failed);
                }
            }
            PowerCommand::Disconnect => {
                if self.process_disconnect(port, controller, power).await.is_err() {
                    error!("Error processing disconnect");
                    return Err(PowerError::Failed);
                }
            }
        }

        Ok(ResponseData::Complete)
    }
}
