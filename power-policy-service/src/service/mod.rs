//! Power policy related data structures and messages
use core::ptr;

pub mod config;
pub mod consumer;
pub mod context;
pub mod event;
pub mod provider;
pub mod task;

use embedded_services::{error, info, sync::Lockable};

use crate::{
    capability::{ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    psu::{
        Error, Psu,
        event::{Event as PsuEvent, EventData as PsuEventData},
    },
    service::event::Event as ServiceEvent,
};

/// Unconstrained state information
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UnconstrainedState {
    /// Unconstrained state
    pub unconstrained: bool,
    /// Available unconstrained devices
    pub available: usize,
}

impl UnconstrainedState {
    /// Create a new unconstrained state
    pub fn new(unconstrained: bool, available: usize) -> Self {
        Self {
            unconstrained,
            available,
        }
    }
}

const MAX_CONNECTED_PROVIDERS: usize = 4;

#[derive(Clone)]
struct InternalState<'device, D: Lockable>
where
    D::Inner: Psu,
{
    /// Current consumer state, if any
    current_consumer_state: Option<consumer::AvailableConsumer<'device, D>>,
    /// Current provider global state
    current_provider_state: provider::State,
    /// System unconstrained power
    unconstrained: UnconstrainedState,
    /// Connected providers
    connected_providers: heapless::FnvIndexSet<*const D, MAX_CONNECTED_PROVIDERS>,
}

impl<D: Lockable> Default for InternalState<'_, D>
where
    D::Inner: Psu,
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
pub struct Service<'a, PSU: Lockable>
where
    PSU::Inner: Psu,
{
    /// Power policy context
    pub context: &'a context::Context,
    /// PSU devices
    psu_devices: &'a [&'a PSU],
    /// State
    state: InternalState<'a, PSU>,
    /// Config
    config: config::Config,
}

impl<'a, PSU: Lockable> Service<'a, PSU>
where
    PSU::Inner: Psu,
{
    /// Create a new power policy
    pub fn new(psu_devices: &'a [&'a PSU], context: &'a context::Context, config: config::Config) -> Self {
        Self {
            context,
            psu_devices,
            state: InternalState::default(),
            config,
        }
    }

    /// Returns the total amount of power that is being supplied to external devices
    pub async fn compute_total_provider_power_mw(&self) -> u32 {
        let mut total = 0;

        for psu in self.psu_devices.iter() {
            let mut psu = psu.lock().await;
            total += psu
                .state()
                .connected_provider_capability()
                .map(|cap| cap.capability.max_power_mw())
                .unwrap_or(0);
        }

        total
    }

    async fn process_notify_attach(&self, device: &PSU) {
        let mut device = device.lock().await;
        info!("({}): Received notify attached", device.name());
        if let Err(e) = device.state().attach() {
            error!("({}): Invalid state for attach: {:#?}", device.name(), e);
        }
    }

    async fn process_notify_detach(&mut self, device: &PSU) -> Result<(), Error> {
        {
            let mut device = device.lock().await;
            info!("({}): Received notify detached", device.name());
            device.state().detach();
        }
        self.update_current_consumer().await
    }

    async fn process_notify_consumer_power_capability(
        &mut self,
        device: &PSU,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        {
            let mut device = device.lock().await;
            info!(
                "({}): Received notify consumer capability: {:#?}",
                device.name(),
                capability,
            );
            if let Err(e) = device.state().update_consumer_power_capability(capability) {
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
        requester: &'a PSU,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        {
            let mut requester = requester.lock().await;
            info!(
                "({}): Received request provider capability: {:#?}",
                requester.name(),
                capability,
            );
            if let Err(e) = requester.state().update_requested_provider_power_capability(capability) {
                error!(
                    "({}): Invalid state for notify consumer capability, catching up: {:#?}",
                    requester.name(),
                    e,
                );
            }
        }

        self.connect_provider(requester).await
    }

    async fn process_notify_disconnect(&mut self, device: &'a PSU) -> Result<(), Error> {
        let mut locked_device = device.lock().await;
        info!("({}): Received notify disconnect", locked_device.name());

        if let Err(e) = locked_device.state().disconnect(true) {
            error!(
                "({}): Invalid state for notify disconnect, catching up: {:#?}",
                locked_device.name(),
                e,
            );
        }

        if self
            .state
            .current_consumer_state
            .as_ref()
            .is_some_and(|current| ptr::eq(current.psu, device))
        {
            info!("({}): Connected consumer disconnected", locked_device.name());
            self.disconnect_chargers().await?;

            self.broadcast_event(ServiceEvent::ConsumerDisconnected(device)).await;
        }

        self.remove_connected_provider(device).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    /// Send an event to all registered listeners
    async fn broadcast_event(&mut self, _message: ServiceEvent<'a, PSU>) {
        // TODO
    }

    /// Common logic for when a provider is disconnected
    ///
    /// Returns true if the device was operating as a provider
    async fn remove_connected_provider(&mut self, psu: &'a PSU) -> bool {
        if self.state.connected_providers.remove(&(psu as *const PSU)) {
            self.broadcast_event(ServiceEvent::ProviderDisconnected(psu)).await;
            true
        } else {
            false
        }
    }

    pub async fn process_psu_event(&mut self, event: PsuEvent<'a, PSU>) -> Result<(), Error> {
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
}
