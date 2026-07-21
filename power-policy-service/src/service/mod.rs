//! Power policy related data structures and messages
use core::ptr;

pub mod config;
pub mod consumer;
pub mod customization;
pub mod provider;
pub mod registration;
pub mod task;

use embedded_services::error;
use embedded_services::named::Named;
use embedded_services::{info, sync::Lockable, trace};

use power_policy_interface::charger::{Charger, PsuState};
use power_policy_interface::psu::notification::NotificationHandler as _;
use power_policy_interface::service::notification::Notifier;
use power_policy_interface::{
    capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability},
    charger::{Event as ChargerEvent, EventData as ChargerEventData},
    psu::{
        Error, Psu,
        event::{Event as PsuEvent, EventData as PsuEventData},
    },
    service::UnconstrainedState,
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
pub struct Service<
    'device,
    Reg: Registration<'device>,
    Customization: customization::Customization = customization::DefaultCustomization,
> {
    /// Service registration
    registration: Reg,
    /// State
    state: InternalState<'device, Reg::Psu>,
    /// Config
    config: config::Config,
    /// Customization
    customization: Customization,
}

impl<'device, Reg: Registration<'device>, Customization: customization::Customization + Default>
    Service<'device, Reg, Customization>
{
    /// Create a new power policy
    pub fn new(registration: Reg, config: config::Config) -> Self {
        Self::new_with_customization(registration, config, Customization::default())
    }
}

impl<'device, Reg: Registration<'device>, Customization: customization::Customization>
    Service<'device, Reg, Customization>
{
    /// Create a new power policy with customization
    pub fn new_with_customization(registration: Reg, config: config::Config, customization: Customization) -> Self {
        Self {
            registration,
            state: InternalState::default(),
            config,
            customization,
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

    async fn notify_unconstrained(&mut self, unconstrained: UnconstrainedState) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_unconstrained(unconstrained).await {
                error!("Failed to notify unconstrained state change: {:#?}", e);
            }
        }
    }

    async fn notify_consumer_connected(&mut self, psu: &'device Reg::Psu, capability: ConsumerPowerCapability) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_consumer_connected(psu, capability).await {
                error!("Failed to notify consumer connected: {:#?}", e);
            }
        }
    }

    async fn notify_consumer_disconnected(&mut self, psu: &'device Reg::Psu, flags: ConsumerDisconnect) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_consumer_disconnected(psu, flags).await {
                error!("Failed to notify consumer disconnected: {:#?}", e);
            }
        }
    }

    async fn notify_provider_connected(&mut self, psu: &'device Reg::Psu, capability: ProviderPowerCapability) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_provider_connected(psu, capability).await {
                error!("Failed to notify provider connected: {:#?}", e);
            }
        }
    }

    async fn notify_provider_disconnected(&mut self, psu: &'device Reg::Psu) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_provider_disconnected(psu).await {
                error!("Failed to notify provider disconnected: {:#?}", e);
            }
        }
    }

    pub async fn process_psu_event(&mut self, event: PsuEvent<'device, Reg::Psu>) -> Result<(), Error> {
        let device = event.psu;
        match event.event {
            PsuEventData::Attached => self.process_notify_attached(device).await,
            PsuEventData::Detached => self.process_notify_detached(device).await,
            PsuEventData::UpdatedConsumerCapability(capability) => {
                self.process_notify_updated_consumer_capability(device, capability)
                    .await
            }
            PsuEventData::RequestedProviderCapability(capability) => {
                self.process_notify_requested_provider_capability(device, capability)
                    .await
            }
            PsuEventData::Disconnected(flags) => self.process_notify_disconnected(device, flags).await,
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

impl<'device, Reg: Registration<'device>, Customization: customization::Customization>
    power_policy_interface::psu::notification::NotificationHandler<'device> for Service<'device, Reg, Customization>
{
    type Psu = Reg::Psu;

    async fn process_notify_attached(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        info!("({}): Received notify attached", device.lock().await.name());
        Ok(())
    }

    async fn process_notify_detached(&mut self, device: &'device Reg::Psu) -> Result<(), Error> {
        info!("({}): Received notify detached", device.lock().await.name());
        self.post_provider_removed(device).await;
        self.update_current_consumer(ConsumerDisconnect::none()).await?;
        Ok(())
    }

    async fn process_notify_updated_consumer_capability(
        &mut self,
        device: &'device Reg::Psu,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "({}): Received notify consumer capability: {:#?}",
            device.lock().await.name(),
            capability,
        );

        self.update_current_consumer(ConsumerDisconnect::none()).await
    }

    async fn process_notify_requested_provider_capability(
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

    async fn process_notify_disconnected(
        &mut self,
        device: &'device Reg::Psu,
        flags: ConsumerDisconnect,
    ) -> Result<(), Error> {
        info!("({}): Received notify disconnect", device.lock().await.name());
        self.post_provider_removed(device).await;
        self.update_current_consumer(flags).await?;
        Ok(())
    }
}
