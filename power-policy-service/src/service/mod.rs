//! Power policy related data structures and messages
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
        DeviceId, Error, Psu,
        event::{Request, RequestData},
    },
    service::event::{CommsData, CommsMessage},
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

#[derive(Clone, Default)]
struct InternalState {
    /// Current consumer state, if any
    current_consumer_state: Option<consumer::AvailableConsumer>,
    /// Current provider global state
    current_provider_state: provider::State,
    /// System unconstrained power
    unconstrained: UnconstrainedState,
    /// Connected providers
    connected_providers: heapless::FnvIndexSet<DeviceId, MAX_CONNECTED_PROVIDERS>,
}

/// Power policy service
pub struct Service<'a, D: Lockable>
where
    D::Inner: Psu,
{
    /// Power policy context
    pub context: &'a context::Context,
    /// PSU devices
    psu_registration: &'a [crate::psu::RegistrationEntry<'a, D>],
    /// State
    state: InternalState,
    /// Config
    config: config::Config,
}

impl<'a, D: Lockable + 'static> Service<'a, D>
where
    D::Inner: Psu,
{
    /// Create a new power policy
    pub fn new(
        psu_registration: &'a [crate::psu::RegistrationEntry<'a, D>],
        context: &'a context::Context,
        config: config::Config,
    ) -> Self {
        Self {
            context,
            psu_registration,
            state: InternalState::default(),
            config,
        }
    }

    /// Get a PSU by its ID
    pub fn get_psu_registration(&self, id: DeviceId) -> Option<&crate::psu::RegistrationEntry<'a, D>> {
        self.psu_registration.iter().find(|psu| psu.id() == id)
    }

    /// Returns the total amount of power that is being supplied to external devices
    pub async fn compute_total_provider_power_mw(&self) -> u32 {
        let mut total = 0;

        for psu_registration in self.psu_registration.iter() {
            let mut psu = psu_registration.device.lock().await;
            total += psu
                .state()
                .connected_provider_capability()
                .map(|cap| cap.capability.max_power_mw())
                .unwrap_or(0);
        }

        total
    }

    async fn process_notify_attach(&self, device_id: DeviceId, device: &D) {
        if let Err(e) = device.lock().await.state().attach() {
            error!("Device{}: Invalid state for attach: {:#?}", device_id.0, e);
        }
    }

    async fn process_notify_detach(&mut self, device: &D) -> Result<(), Error> {
        device.lock().await.state().detach();
        self.update_current_consumer().await
    }

    async fn process_notify_consumer_power_capability(
        &mut self,
        device_id: DeviceId,
        device: &D,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        if let Err(e) = device.lock().await.state().update_consumer_power_capability(capability) {
            error!(
                "Device{}: Invalid state for notify consumer capability, catching up: {:#?}",
                device_id.0, e,
            );
        }

        self.update_current_consumer().await
    }

    async fn process_request_provider_power_capabilities(
        &mut self,
        device_id: DeviceId,
        device: &D,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        if let Err(e) = device
            .lock()
            .await
            .state()
            .update_requested_provider_power_capability(capability)
        {
            error!(
                "Device{}: Invalid state for notify consumer capability, catching up: {:#?}",
                device_id.0, e,
            );
        }

        self.connect_provider(device_id).await
    }

    async fn process_notify_disconnect(&mut self, device_id: DeviceId, device: &D) -> Result<(), Error> {
        if let Err(e) = device.lock().await.state().disconnect(true) {
            error!(
                "Device{}: Invalid state for notify disconnect, catching up: {:#?}",
                device_id.0, e,
            );
        }

        if self
            .state
            .current_consumer_state
            .is_some_and(|current| current.device_id == device_id)
        {
            info!("Device{}: Connected consumer disconnected", device_id.0);
            self.disconnect_chargers().await?;

            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerDisconnected(device_id),
            })
            .await;
        }

        self.remove_connected_provider(device_id).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    /// Send a notification with the comms service
    async fn comms_notify(&self, message: CommsMessage) {
        self.context.broadcast_message(message).await;
    }

    /// Common logic for when a provider is disconnected
    ///
    /// Returns true if the device was operating as a provider
    async fn remove_connected_provider(&mut self, device_id: DeviceId) -> bool {
        if self.state.connected_providers.remove(&device_id) {
            self.comms_notify(CommsMessage {
                data: CommsData::ProviderDisconnected(device_id),
            })
            .await;
            true
        } else {
            false
        }
    }

    pub async fn process_psu_event(&mut self, request: Request) -> Result<(), Error> {
        let device = self
            .get_psu_registration(request.id)
            .ok_or(Error::InvalidDevice)?
            .device;

        match request.data {
            RequestData::Attached => {
                info!("Received notify attached from device {}", request.id.0);
                self.process_notify_attach(request.id, device).await;
                Ok(())
            }
            RequestData::Detached => {
                info!("Received notify detached from device {}", request.id.0);
                self.process_notify_detach(device).await
            }
            RequestData::UpdatedConsumerCapability(capability) => {
                info!(
                    "Device{}: Received notify consumer capability: {:#?}",
                    request.id.0, capability,
                );
                self.process_notify_consumer_power_capability(request.id, device, capability)
                    .await
            }
            RequestData::RequestedProviderCapability(capability) => {
                info!(
                    "Device{}: Received request provider capability: {:#?}",
                    request.id.0, capability,
                );
                self.process_request_provider_power_capabilities(request.id, device, capability)
                    .await
            }
            RequestData::Disconnected => {
                info!("Received notify disconnect from device {}", request.id.0);
                self.process_notify_disconnect(request.id, device).await
            }
        }
    }
}
