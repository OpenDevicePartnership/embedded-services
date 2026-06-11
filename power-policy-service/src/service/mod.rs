//! Power policy related data structures and messages
use core::ptr;

pub mod config;
pub mod consumer;
pub mod hooks;
pub mod provider;
pub mod registration;
pub mod task;

use embedded_services::error;
use embedded_services::named::Named;
use embedded_services::{event::NonBlockingSender, info, sync::Lockable, trace};

use power_policy_interface::charger::{Charger, PsuState};
use power_policy_interface::{
    capability::{ConsumerPowerCapability, ProviderPowerCapability},
    charger::{Event as ChargerEvent, EventData as ChargerEventData},
    psu::{
        Error, Psu,
        event::{Event as PsuEvent, EventData as PsuEventData},
    },
    service::{UnconstrainedState, event::Event as ServiceEvent},
};

use crate::service::registration::Registration;

const MAX_CONNECTED_PROVIDERS: usize = 4;

#[derive(Clone)]
pub struct InternalState<'device, PSU: Lockable>
where
    PSU::Inner: Psu,
{
    /// Current consumer state, if any
    pub current_consumer_state: Option<consumer::AvailableConsumer<'device, PSU>>,
    /// Current provider global state
    pub current_provider_state: provider::State,
    /// System unconstrained power
    pub unconstrained: UnconstrainedState,
    /// Connected providers
    pub connected_providers: heapless::index_set::FnvIndexSet<usize, MAX_CONNECTED_PROVIDERS>,
}

impl<PSU: Lockable> Default for InternalState<'_, PSU>
where
    PSU::Inner: Psu,
{
    fn default() -> Self {
        Self {
            current_consumer_state: None,
            current_provider_state: provider::State::default(),
            unconstrained: UnconstrainedState::default(),
            connected_providers: heapless::index_set::FnvIndexSet::new(),
        }
    }
}

/// Power policy service
pub struct Service<'device, Reg: Registration<'device>, Hooks: hooks::Hooks<'device, Reg> = hooks::DefaultHooks> {
    /// Service registration
    registration: Reg,
    /// State
    state: InternalState<'device, Reg::Psu>,
    /// Config
    config: config::Config,
    /// Hooks
    hooks: Hooks,
}

impl<'device, Reg: Registration<'device>, Hooks: hooks::Hooks<'device, Reg> + Default> Service<'device, Reg, Hooks> {
    /// Create a new power policy
    pub fn new(registration: Reg, config: config::Config) -> Self {
        Self::new_with_hooks(registration, config, Hooks::default())
    }
}

impl<'device, Reg: Registration<'device>, Hooks: hooks::Hooks<'device, Reg>> Service<'device, Reg, Hooks> {
    /// Create a new power policy with custom hooks
    pub fn new_with_hooks(registration: Reg, config: config::Config, hooks: Hooks) -> Self {
        Self {
            registration,
            state: InternalState::default(),
            config,
            hooks,
        }
    }

    /// Returns the total amount of power that is being supplied to external devices
    pub async fn compute_total_provider_power_mw(&self) -> u32 {
        let mut total = 0;

        for psu in self.registration.psus() {
            let psu = psu.lock().await;
            total += psu
                .state()
                .connected_provider_capability()
                .map(|cap| cap.capability.max_power_mw())
                .unwrap_or(0);
        }

        total
    }

    async fn process_notify_attach(&self, device: &'device Reg::Psu) {
        info!("({}): Received notify attached", device.lock().await.name());
    }

    async fn process_notify_detach(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        info!("({}): Received notify detached", device.lock().await.name());
        self.post_provider_removed(device).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    async fn process_notify_consumer_power_capability(
        &mut self,
        device: &'device Reg::Psu,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "({}): Received notify consumer capability: {:#?}",
            device.lock().await.name(),
            capability,
        );

        self.update_current_consumer().await
    }

    async fn process_request_provider_power_capabilities(
        &mut self,
        requester: &'device Reg::Psu,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "({}): Received request provider capability: {:#?}",
            requester.lock().await.name(),
            capability,
        );

        self.connect_provider(requester).await
    }

    async fn process_notify_disconnect(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        info!("({}): Received notify disconnect", device.lock().await.name());
        self.post_provider_removed(device).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    /// Send an event to all registered listeners
    fn broadcast_event(&mut self, event: ServiceEvent<'device, Reg::Psu>) {
        for sender in self.registration.event_senders() {
            if sender.try_send(event).is_none() {
                error!("Failed to send event to listener");
            }
        }
    }

    pub async fn process_psu_event(&mut self, event: PsuEvent<'device, Reg::Psu>) -> Result<(), Error> {
        let device = event.psu;
        match event.event {
            PsuEventData::Attached => {
                self.process_notify_attach(device).await;
                Ok(())
            }
            PsuEventData::Detached => self.process_notify_detach(device).await,
            PsuEventData::UpdatedConsumerCapability(capability) => {
                self.process_notify_consumer_power_capability(device, capability).await
            }
            PsuEventData::RequestedProviderCapability(capability) => {
                self.process_request_provider_power_capabilities(device, capability)
                    .await
            }
            PsuEventData::Disconnected => self.process_notify_disconnect(device).await,
            _ => {
                info!(
                    "Received unknown PSU event from ({}): {:?}",
                    device.lock().await.name(),
                    event.event
                );
                Ok(())
            }
        }
    }

    async fn process_psu_state_change(
        &mut self,
        charger: &'device Reg::Charger,
        psu_state: PsuState,
    ) -> Result<(), Error> {
        // Currently a no-op, but functionality might be added in the future.
        let locked_charger = charger.lock().await;
        trace!(
            "Charger PSU state change to {:?} event recvd in charger state {:?}",
            psu_state,
            locked_charger.state()
        );
        Ok(())
    }

    pub async fn process_charger_event(&mut self, event: ChargerEvent<'device, Reg::Charger>) -> Result<(), Error> {
        let charger = event.charger;

        match event.event {
            ChargerEventData::PsuStateChange(psu_state) => self.process_psu_state_change(charger, psu_state).await?,
            _ => {
                return Err(Error::Charger(
                    power_policy_interface::charger::ChargerError::UnknownEvent,
                ));
            }
        };
        Ok(())
    }
}
