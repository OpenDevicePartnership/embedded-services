//! Module contain power-policy related message handling
use embedded_services::debug;

use crate::wrapper::config::UnconstrainedSink;
use power_policy_interface::capability::{ConsumerPowerCapability, ProviderPowerCapability, PsuType};
use power_policy_interface::psu::CommandData as PowerCommand;
use power_policy_interface::psu::Error as PowerError;
use power_policy_interface::psu::{CommandData, InternalResponseData, ResponseData};

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
    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(
        &self,
        port_state: &mut PortState<S>,
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

        port_state
            .power_policy_sender
            .send(power_policy_interface::psu::event::EventData::UpdatedConsumerCapability(available_sink_contract))
            .await;
        Ok(())
    }

    /// Handle a new contract as provider
    pub(super) async fn process_new_provider_contract(
        &self,
        port_state: &mut PortState<S>,
        status: &PortStatus,
    ) -> Result<(), Error<<D::Inner as Controller>::BusError>> {
        info!("Process New provider contract");
        port_state
            .power_policy_sender
            .send(
                power_policy_interface::psu::event::EventData::RequestedProviderCapability(
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

    /// Process a power command
    /// Returns no error because this is a top-level function
    pub(super) async fn process_power_command(
        &self,
        cfu_event_receiver: &mut CfuEventReceiver,
        controller: &mut D::Inner,
        port: LocalPortId,
        command: &CommandData,
    ) -> InternalResponseData {
        trace!("Processing power command: device{} {:#?}", port.0, command);
        if cfu_event_receiver.fw_update_state.in_progress() {
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
