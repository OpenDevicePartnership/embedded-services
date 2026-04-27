//! Power policy related data structures and messages
use core::ptr;

pub mod config;
pub mod consumer;
pub mod provider;
pub mod registration;
pub mod task;

use embedded_services::named::Named;
use embedded_services::trace;
use embedded_services::{error, event::Sender, info, sync::Lockable};

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
struct InternalState<'device, PSU: Lockable>
where
    PSU::Inner: Psu,
{
    /// Current consumer state, if any
    current_consumer_state: Option<consumer::AvailableConsumer<'device, PSU>>,
    /// Current provider global state
    current_provider_state: provider::State,
    /// System unconstrained power
    unconstrained: UnconstrainedState,
    /// Connected providers
    connected_providers: heapless::FnvIndexSet<usize, MAX_CONNECTED_PROVIDERS>,
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
            connected_providers: heapless::FnvIndexSet::new(),
        }
    }
}

/// Power policy service
pub struct Service<'device, Reg: Registration<'device>> {
    /// Service registration
    registration: Reg,
    /// State
    state: InternalState<'device, Reg::Psu>,
    /// Config
    config: config::Config,
}

impl<'device, Reg: Registration<'device>> Service<'device, Reg> {
    /// Create a new power policy
    pub fn new(registration: Reg, config: config::Config) -> Self {
        Self {
            registration,
            state: InternalState::default(),
            config,
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
        let mut device = device.lock().await;
        info!("({}): Received notify attached", device.name());
        if let Err(e) = device.state_mut().attach() {
            error!("({}): Invalid state for attach: {:#?}", device.name(), e);
        }
    }

    async fn process_notify_detach(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        {
            let mut device = device.lock().await;
            info!("({}): Received notify detached", device.name());
            device.state_mut().detach();
        }

        self.post_provider_removed(device).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    async fn process_notify_consumer_power_capability(
        &mut self,
        device: &'device Reg::Psu,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        {
            let mut device = device.lock().await;
            info!(
                "({}): Received notify consumer capability: {:#?}",
                device.name(),
                capability,
            );
            if let Err(e) = device.state_mut().update_consumer_power_capability(capability) {
                error!(
                    "({}): Invalid state for notify consumer capability, catching up: {:#?}",
                    device.name(),
                    e,
                );
            }
        }

        self.update_current_consumer().await
    }

    async fn process_request_provider_power_capabilities(
        &mut self,
        requester: &'device Reg::Psu,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        {
            let mut requester = requester.lock().await;
            info!(
                "({}): Received request provider capability: {:#?}",
                requester.name(),
                capability,
            );
            if let Err(e) = requester
                .state_mut()
                .update_requested_provider_power_capability(capability)
            {
                error!(
                    "({}): Invalid state for notify provider capability, catching up: {:#?}",
                    requester.name(),
                    e,
                );
            }
        }

        self.connect_provider(requester).await
    }

    async fn process_notify_disconnect(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        {
            let mut locked_device = device.lock().await;
            info!("({}): Received notify disconnect", locked_device.name());

            if let Err(e) = locked_device.state_mut().disconnect(true) {
                error!(
                    "({}): Invalid state for notify disconnect, catching up: {:#?}",
                    locked_device.name(),
                    e,
                );
            }
        }

        self.post_provider_removed(device).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    /// Send an event to all registered listeners
    async fn broadcast_event(&mut self, event: ServiceEvent<'device, Reg::Psu>) {
        for sender in self.registration.event_senders() {
            sender.send(event).await;
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
