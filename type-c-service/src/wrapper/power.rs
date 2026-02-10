//! Module contain power-policy related message handling
use core::pin::pin;

use embassy_futures::select::select_slice;
use embedded_services::debug;

use power_policy_service::capability::{ConsumerPowerCapability, ProviderPowerCapability, PsuType};
use power_policy_service::device::CommandData as PowerCommand;
use power_policy_service::device::Error as PowerError;
use power_policy_service::device::{CommandData, InternalResponseData, ResponseData};

use crate::wrapper::config::UnconstrainedSink;

use super::*;

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_service::device::event::RequestData>,
    R: event::Receiver<power_policy_service::device::event::RequestData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, R, V>
where
    D::Inner: Controller,
{
    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(
        &self,
        power: &mut PortPower<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        info!("Process new consumer contract");
        let available_sink_contract = status.available_sink_contract.map(|c| {
            let mut c: ConsumerPowerCapability = c.into();
            let unconstrained = match self.config.unconstrained_sink {
                UnconstrainedSink::Auto => status.unconstrained_power,
                UnconstrainedSink::PowerThresholdMilliwatts(threshold) => c.capability.max_power_mw() >= threshold,
                UnconstrainedSink::Never => false,
            };
            c.flags.set_unconstrained_power(unconstrained);
            c.flags.set_psu_type(PsuType::TypeC);
            c
        });

        power
            .sender
            .send(power_policy_service::device::event::RequestData::UpdatedConsumerCapability(available_sink_contract))
            .await;
        Ok(())
    }

    /// Handle a new contract as provider
    pub(super) async fn process_new_provider_contract(
        &self,
        power: &mut PortPower<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        info!("Process New provider contract");
        power
            .sender
            .send(
                power_policy_service::device::event::RequestData::RequestedProviderCapability(
                    status.available_source_contract.map(|caps| {
                        let mut caps = ProviderPowerCapability::from(caps);
                        caps.flags.set_psu_type(PsuType::TypeC);
                        caps
                    }),
                ),
            )
            .await;
        Ok(())
    }

    /// Handle a disconnect command
    async fn process_disconnect(
        &self,
        port: LocalPortId,
        controller: &mut D::Inner,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        if controller.enable_sink_path(port, false).await.is_err() {
            error!("Error disabling sink path");
            return PdError::Failed.into();
        }
        Ok(())
    }

    /// Handle a connect as provider command
    fn process_connect_as_provider(
        &self,
        port: LocalPortId,
        capability: ProviderPowerCapability,
        _controller: &mut D::Inner,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
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
            // TODO: check this at compile time
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
        controller: &mut D::Inner,
        state: &mut dyn DynPortState<'_, S>,
        port: LocalPortId,
        command: &CommandData,
    ) -> InternalResponseData {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        if state.controller_state().fw_update_state.in_progress() {
            debug!("Port{}: Firmware update in progress", port.0);
            return Err(PowerError::Busy);
        }

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
                if self.process_connect_as_provider(port, *capability, controller).is_err() {
                    error!("Error processing connect provider");
                    return Err(PowerError::Failed);
                }
            }
            PowerCommand::Disconnect => {
                if self.process_disconnect(port, controller).await.is_err() {
                    error!("Error processing disconnect");
                    return Err(PowerError::Failed);
                }
            }
        }

        Ok(ResponseData::Complete)
    }
}
