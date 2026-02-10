//! Power policy related data structures and messages
pub mod config;
pub mod consumer;
pub mod context;
pub mod event;
pub mod provider;
pub mod task;

pub use context::init;
use embassy_sync::mutex::Mutex;
use embedded_services::{GlobalRawMutex, comms, error, event::Receiver, info, sync::Lockable};

use crate::{
    capability::{ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    psu::{
        DeviceId, Error, Psu, RegistrationEntry,
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
pub struct Service<'a, D: Lockable, R: Receiver<RequestData>>
where
    D::Inner: Psu,
{
    /// Power policy context
    pub context: &'a context::Context<D, R>,
    /// State
    state: Mutex<GlobalRawMutex, InternalState>,
    /// Comms endpoint
    tp: comms::Endpoint,
    /// Config
    config: config::Config,
}

impl<'a, D: Lockable + 'static, R: Receiver<RequestData> + 'static> Service<'a, D, R>
where
    D::Inner: Psu,
{
    /// Create a new power policy
    pub fn new(context: &'a context::Context<D, R>, config: config::Config) -> Self {
        Self {
            context,
            state: Mutex::new(InternalState::default()),
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Power)),
            config,
        }
    }

    async fn process_notify_attach(&self, device: &RegistrationEntry<'_, D, R>) {
        if let Err(e) = device.state.lock().await.attach() {
            error!("Device{}: Invalid state for attach: {:#?}", device.id().0, e);
        }
    }

    async fn process_notify_detach(&self, device: &RegistrationEntry<'_, D, R>) -> Result<(), Error> {
        device.state.lock().await.detach();
        self.update_current_consumer().await
    }

    async fn process_notify_consumer_power_capability(
        &self,
        device: &RegistrationEntry<'_, D, R>,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        if let Err(e) = device.state.lock().await.update_consumer_power_capability(capability) {
            error!(
                "Device{}: Invalid state for notify consumer capability, catching up: {:#?}",
                device.id().0,
                e,
            );
        }

        self.update_current_consumer().await
    }

    async fn process_request_provider_power_capabilities(
        &self,
        device: &RegistrationEntry<'_, D, R>,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        if let Err(e) = device
            .state
            .lock()
            .await
            .update_requested_provider_power_capability(capability)
        {
            error!(
                "Device{}: Invalid state for notify consumer capability, catching up: {:#?}",
                device.id().0,
                e,
            );
        }

        self.connect_provider(device.id()).await
    }

    async fn process_notify_disconnect(&self, device: &RegistrationEntry<'_, D, R>) -> Result<(), Error> {
        if let Err(e) = device.state.lock().await.disconnect(true) {
            error!(
                "Device{}: Invalid state for notify disconnect, catching up: {:#?}",
                device.id().0,
                e,
            );
        }

        if self
            .state
            .lock()
            .await
            .current_consumer_state
            .is_some_and(|current| current.device_id == device.id())
        {
            info!("Device{}: Connected consumer disconnected", device.id().0);
            self.disconnect_chargers().await?;

            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerDisconnected(device.id()),
            })
            .await;
        }

        self.remove_connected_provider(device.id()).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    /// Send a notification with the comms service
    async fn comms_notify(&self, message: CommsMessage) {
        self.context.broadcast_message(message).await;
        let _ = self
            .tp
            .send(comms::EndpointID::Internal(comms::Internal::Battery), &message)
            .await;
    }

    /// Common logic for when a provider is disconnected
    ///
    /// Returns true if the device was operating as a provider
    async fn remove_connected_provider(&self, device_id: DeviceId) -> bool {
        if self.state.lock().await.connected_providers.remove(&device_id) {
            self.comms_notify(CommsMessage {
                data: CommsData::ProviderDisconnected(device_id),
            })
            .await;
            true
        } else {
            false
        }
    }

    async fn wait_request(&self) -> Request {
        self.context.wait_request().await
    }

    async fn process_request(&self, request: Request) -> Result<(), Error> {
        let device = self.context.get_psu(request.id)?;

        match request.data {
            RequestData::Attached => {
                info!("Received notify attached from device {}", device.id().0);
                self.process_notify_attach(device).await;
                Ok(())
            }
            RequestData::Detached => {
                info!("Received notify detached from device {}", device.id().0);
                self.process_notify_detach(device).await
            }
            RequestData::UpdatedConsumerCapability(capability) => {
                info!(
                    "Device{}: Received notify consumer capability: {:#?}",
                    device.id().0,
                    capability,
                );
                self.process_notify_consumer_power_capability(device, capability).await
            }
            RequestData::RequestedProviderCapability(capability) => {
                info!(
                    "Device{}: Received request provider capability: {:#?}",
                    device.id().0,
                    capability,
                );
                self.process_request_provider_power_capabilities(device, capability)
                    .await
            }
            RequestData::Disconnected => {
                info!("Received notify disconnect from device {}", device.id().0);
                self.process_notify_disconnect(device).await
            }
        }
    }

    /// Top-level event loop function
    pub async fn process(&self) -> Result<(), Error> {
        let request = self.wait_request().await;
        self.process_request(request).await
    }
}

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> comms::MailboxDelegate for Service<'_, D, R> where
    D::Inner: Psu
{
}
