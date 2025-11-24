#![no_std]
use core::ops::DerefMut;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::GlobalRawMutex;
use embedded_services::power::policy::device::Device;
use embedded_services::power::policy::{action, policy, *};
use embedded_services::{comms, error, info};

pub mod config;
pub mod consumer;
pub mod provider;

pub use config::Config;
pub mod charger;

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

/// Power policy state
pub struct PowerPolicy {
    /// Power policy context
    context: policy::ContextToken,
    /// State
    state: Mutex<GlobalRawMutex, InternalState>,
    /// Comms endpoint
    tp: comms::Endpoint,
    /// Config
    config: config::Config,
}

impl PowerPolicy {
    /// Create a new power policy
    pub fn create(config: config::Config) -> Option<Self> {
        Some(Self {
            context: policy::ContextToken::create()?,
            state: Mutex::new(InternalState::default()),
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Power)),
            config,
        })
    }

    async fn process_notify_attach(&self) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_detach(&self, device: &device::Device) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        if !self.remove_connected_provider(device.id()).await {
            // Only update consumers if a consumer was detached
            self.update_current_consumer().await?;
        }
        Ok(())
    }

    async fn process_notify_consumer_power_capability(&self) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        self.update_current_consumer().await?;
        Ok(())
    }

    async fn process_request_provider_power_capabilities(&self, device: DeviceId) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        self.connect_provider(device).await;
        Ok(())
    }

    async fn process_notify_disconnect(&self, device: &device::Device) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        if let Some(consumer) = self.state.lock().await.current_consumer_state.take() {
            info!("Device{}: Connected consumer disconnected", consumer.device_id.0);
            self.disconnect_chargers().await?;

            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerDisconnected(consumer.device_id),
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

    async fn wait_request(&self) -> policy::Request {
        self.context.wait_request().await
    }

    async fn process_request(&self, request: policy::Request) -> Result<(), Error> {
        let device = self.context.get_device(request.id)?;

        match request.data {
            policy::RequestData::NotifyAttached => {
                info!("Received notify attached from device {}", device.id().0);
                self.process_notify_attach().await
            }
            policy::RequestData::NotifyDetached => {
                info!("Received notify detached from device {}", device.id().0);
                self.process_notify_detach(device).await
            }
            policy::RequestData::NotifyConsumerCapability(capability) => {
                info!(
                    "Device{}: Received notify consumer capability: {:#?}",
                    device.id().0,
                    capability,
                );
                self.process_notify_consumer_power_capability().await
            }
            policy::RequestData::RequestProviderCapability(capability) => {
                info!(
                    "Device{}: Received request provider capability: {:#?}",
                    device.id().0,
                    capability,
                );
                self.process_request_provider_power_capabilities(device.id()).await
            }
            policy::RequestData::NotifyDisconnect => {
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

impl comms::MailboxDelegate for PowerPolicy {}

#[embassy_executor::task]
pub async fn task(config: config::Config) {
    info!("Starting power policy task");
    static POLICY: OnceLock<PowerPolicy> = OnceLock::new();
    let policy =
        POLICY.get_or_init(|| PowerPolicy::create(config).expect("Power policy singleton already initialized"));

    if comms::register_endpoint(policy, &policy.tp).await.is_err() {
        error!("Failed to register power policy endpoint");
        return;
    }

    loop {
        if let Err(e) = policy.process().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
