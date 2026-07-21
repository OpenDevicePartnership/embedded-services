//! Module for power policy related functionality
use embassy_time::{Duration, Instant};
use embedded_services::{debug, error, event::NonBlockingSender, info, sync::Lockable};
use embedded_usb_pd::{
    PdError,
    constants::{T_PS_TRANSITION_EPR_MS, T_PS_TRANSITION_SPR_MS},
};
use power_policy_interface::{
    capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability, PsuType},
    psu::{Error as PsuError, Psu, State},
};
use type_c_interface::controller::power::SystemPowerStateStatus;

use crate::controller::config::UnconstrainedSink;
use type_c_interface::util::power_policy_error_from_pd_error;

use super::*;

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> Port<'device, C, Shared, TypeCSender, PowerNotifier, LoopbackSender>
{
    /// Handle a new contract as consumer
    pub(super) async fn process_new_consumer_contract(&mut self, new_status: &PortStatus) -> Result<(), PdError> {
        info!("Process new consumer contract");
        let available_sink_contract = new_status.available_sink_contract.map(|c| {
            let mut c: ConsumerPowerCapability = c.into();
            let unconstrained = match self.config.unconstrained_sink {
                UnconstrainedSink::Auto => new_status.unconstrained_power,
                UnconstrainedSink::PowerThresholdMilliwatts(threshold) => c.capability.max_power_mw() >= threshold,
                UnconstrainedSink::Never => false,
            };
            c.flags.set_unconstrained_power(unconstrained);
            c.flags.set_psu_type(PsuType::TypeC);
            c
        });

        if let Err(e) = self.psu_state.update_consumer_power_capability(available_sink_contract) {
            error!("Failed to update consumer power capability: {:?}", e);
            return Err(PdError::Failed);
        }

        if let Err(e) = self
            .power_policy_notifier
            .notify_updated_consumer_capability(available_sink_contract)
            .await
        {
            error!(
                "({}): Failed to notify power policy of updated consumer capability: {:#?}",
                self.name, e
            );
        }
        Ok(())
    }

    /// Handle a new contract as provider
    pub(super) async fn process_new_provider_contract(&mut self, new_status: &PortStatus) -> Result<(), PdError> {
        info!("Process New provider contract");
        let capability = new_status.available_source_contract.map(|caps| {
            let mut caps = ProviderPowerCapability::from(caps);
            caps.flags.set_psu_type(PsuType::TypeC);
            caps
        });
        if let Err(e) = self.psu_state.update_requested_provider_power_capability(capability) {
            error!("Failed to update requested provider power capability: {:?}", e);
            return Err(PdError::Failed);
        }

        if let Err(e) = self
            .power_policy_notifier
            .notify_requested_provider_capability(capability)
            .await
        {
            error!(
                "({}): Failed to notify power policy of requested provider capability: {:#?}",
                self.name, e
            );
        }
        Ok(())
    }

    /// Handle a power role swap
    ///
    /// Called on a `power_swap_completed` event. The power policy rejects a new-role connection
    /// while the device is still tracked in its old connected role, so tear the old role down first:
    /// disable the sink path if we were consuming, reset the local PSU state, and notify the power
    /// policy of the disconnect. The following contract event then connects the new role.
    pub(super) async fn process_power_role_swap(&mut self, new_status: &PortStatus) -> Result<(), PdError> {
        // Only act while the port stays connected.
        if !new_status.is_connected() {
            return Ok(());
        }

        // Nothing to tear down unless we're currently connected in a power role.
        let was_consumer = match self.psu_state.psu_state {
            PsuState::ConnectedConsumer(_) => true,
            PsuState::ConnectedProvider(_) => false,
            _ => return Ok(()),
        };

        info!(
            "({}): Power role swap completed, tearing down previous contract",
            self.name
        );

        // Disable the sink path if we were consuming power so the source can safely stop.
        if was_consumer {
            self.controller.lock().await.enable_sink_path(self.port, false).await?;
        }

        // Reset our local state and notify the power policy so it stops tracking us in the previous
        // role and broadcasts the matching disconnect event.
        if let Err(e) = self.psu_state.disconnect(true) {
            error!("({}): Error updating PSU state on role swap: {:?}", self.name, e);
        }
        if let Err(e) = self
            .power_policy_notifier
            .notify_disconnected(ConsumerDisconnect::none())
            .await
        {
            error!(
                "({}): Failed to notify power policy of role swap disconnect: {:#?}",
                self.name, e
            );
        }

        Ok(())
    }

    /// Check the sink ready timeout
    ///
    /// After accepting a sink contract (new contract as consumer), the PD spec guarantees that the
    /// source will be available to provide power after `tPSTransition`. This allows us to handle transitions
    /// even for controllers that might not always broadcast sink ready events.
    pub(super) async fn check_sink_ready_timeout(
        &mut self,
        new_status: &PortStatus,
        new_contract: bool,
        sink_ready: bool,
    ) -> Result<(), PdError> {
        let contract_changed = self.status.available_sink_contract != new_status.available_sink_contract;
        let mut shared_state = self.shared_state.lock().await;
        let timeout = &mut shared_state.sink_ready_timeout;

        // Don't start the timeout if the sink has signaled it's ready or if the contract didn't change.
        // The latter ensures that soft resets won't continually reset the ready timeout
        debug!(
            "({}): Check sink ready: new_contract={:?}, sink_ready={:?}, contract_changed={:?}, deadline={:?}",
            self.name, new_contract, sink_ready, contract_changed, timeout,
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

            debug!("({}): Sink ready timeout started for {}ms", self.name, timeout_ms);
            *timeout = Some(Instant::now() + Duration::from_millis(timeout_ms as u64));
        } else if timeout.is_some()
            && (!new_status.is_connected() || new_status.available_sink_contract.is_none() || sink_ready)
        {
            debug!("({}): Sink ready timeout cleared", self.name);
            *timeout = None;
        }
        Ok(())
    }
}

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> Psu for Port<'device, C, Shared, TypeCSender, PowerNotifier, LoopbackSender>
{
    async fn disconnect(&mut self) -> Result<(), PsuError> {
        self.controller
            .lock()
            .await
            .enable_sink_path(self.port, false)
            .await
            .map_err(|e| {
                error!("({}): Error disabling sink path", self.name);
                power_policy_error_from_pd_error(e)
            })?;
        self.psu_state.disconnect(false)
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), PsuError> {
        info!("({}): Connect as provider: {:#?}", self.name, capability);
        // TODO: Implement controller over provider enablement
        self.psu_state.connect_provider(capability).inspect_err(|e| {
            error!("({}): Failed to transition to provider state: {:#?}", self.name, e);
        })
    }

    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), PsuError> {
        info!(
            "({}): Connect as consumer: {:?}, enable input switch",
            self.name, capability
        );
        self.controller
            .lock()
            .await
            .enable_sink_path(self.port, true)
            .await
            .map_err(|e| {
                error!("({}): Error enabling sink path", self.name);
                power_policy_error_from_pd_error(e)
            })?;
        self.psu_state.connect_consumer(capability)
    }

    fn state(&self) -> &State {
        &self.psu_state
    }

    fn state_mut(&mut self) -> &mut State {
        &mut self.psu_state
    }
}

impl<
    'device,
    C: Lockable<Inner: Pd + SystemPowerStateStatus>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::power::SystemPowerStateStatus
    for Port<'device, C, Shared, TypeCSender, PowerNotifier, LoopbackSender>
{
    async fn set_system_power_state_status(
        &mut self,
        state: type_c_interface::control::power::SystemPowerState,
    ) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_system_power_state_status(self.port, state)
            .await
    }
}
