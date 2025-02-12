#![no_std]
use core::ops::DerefMut;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::power::policy::device::Device;
use embedded_services::power::policy::{action, policy, *};
use embedded_services::{comms, error, info};

/// State of the current consumer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsumerState {
    /// The ID of the currently connected consumer
    device_id: DeviceId,
    /// The power capability of the currently connected consumer
    power_capability: PowerCapability,
}

impl PartialOrd for ConsumerState {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.power_capability.cmp(&other.power_capability))
    }
}

impl Ord for ConsumerState {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.power_capability.cmp(&other.power_capability)
    }
}

struct InternalState {
    /// Current consumer state, if any
    current_consumer_state: Option<ConsumerState>,
}

impl InternalState {
    fn new() -> Self {
        Self {
            current_consumer_state: None,
        }
    }
}

/// Power policy state
pub struct PowerPolicy {
    /// Power policy context
    context: policy::ContextToken,
    /// State
    state: Mutex<NoopRawMutex, InternalState>,
    /// Comms endpoint
    tp: comms::Endpoint,
}

impl PowerPolicy {
    /// Create a new power policy
    pub fn create() -> Option<Self> {
        Some(Self {
            context: policy::ContextToken::create()?,
            state: Mutex::new(InternalState::new()),
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Power)),
        })
    }

    async fn process_notify_attach(&self) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_detach(&self) -> Result<(), Error> {
        self.update_current_consumer().await?;
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_consumer_power_capability(&self) -> Result<(), Error> {
        self.update_current_consumer().await?;
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_request_provider_power_capabilities(&self) -> Result<(), Error> {
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_disconnect(&self) -> Result<(), Error> {
        self.update_current_consumer().await?;
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    /// Send a notification with the comms service
    async fn comms_notify(&self, message: CommsMessage) {
        let _ = self
            .tp
            .send(comms::EndpointID::Internal(comms::Internal::Battery), &message)
            .await;
    }

    /// Iterate over all devices to determine what is now the highest-powered consumer
    async fn find_highest_power_consumer(&self) -> Result<Option<ConsumerState>, Error> {
        let mut best_consumer = None;

        for node in self.context.devices().await {
            let device = node.data::<Device>().ok_or(Error::InvalidDevice)?;

            // Update the best available consumer
            best_consumer = match (best_consumer, device.consumer_capability().await) {
                // Nothing available
                (None, None) => None,
                // No existing consumer
                (None, Some(power_capability)) => Some(ConsumerState {
                    device_id: device.id(),
                    power_capability,
                }),
                // Existing consumer, no new consumer
                (Some(_), None) => best_consumer,
                // Existing consumer, new available consumer
                (Some(best), Some(available)) => {
                    if available > best.power_capability {
                        Some(ConsumerState {
                            device_id: device.id(),
                            power_capability: available,
                        })
                    } else {
                        best_consumer
                    }
                }
            };
        }

        Ok(best_consumer)
    }

    /// Connect to a new consumer
    async fn connect_new_consumer(&self, state: &mut InternalState, new_consumer: ConsumerState) -> Result<(), Error> {
        // Handle our current consumer
        if let Some(current_consumer) = state.current_consumer_state {
            if new_consumer.device_id == current_consumer.device_id
                && new_consumer.power_capability == current_consumer.power_capability
            {
                // If the consumer is the same device, capability, and is still available, we don't need to do anything
                info!("Best consumer is the same, not switching");
                return Ok(());
            }

            state.current_consumer_state = None;
            // Disconnect the current consumer if needed
            if let Ok(consumer) = self
                .context
                .try_policy_action::<action::ConnectedConsumer>(current_consumer.device_id)
                .await
            {
                info!(
                    "Device {}, disconnecting current consumer",
                    current_consumer.device_id.0
                );
                consumer.disconnect().await?;
            }

            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerConnected(current_consumer.device_id, new_consumer.power_capability),
            })
            .await;
        }

        info!("Device {}, connecting new consumer", new_consumer.device_id.0);
        if let Ok(idle) = self
            .context
            .try_policy_action::<action::Idle>(new_consumer.device_id)
            .await
        {
            idle.connect_consumer(new_consumer.power_capability).await?;
            state.current_consumer_state = Some(new_consumer);
            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerConnected(new_consumer.device_id, new_consumer.power_capability),
            })
            .await;
        } else {
            error!("Error obtaining device in idle state");
        }

        Ok(())
    }

    /// Determines and connects the best consumer
    async fn update_current_consumer(&self) -> Result<(), Error> {
        let mut guard = self.state.lock().await;
        let state = guard.deref_mut();
        info!(
            "Selecting consumer, current consumer: {:#?}",
            state.current_consumer_state
        );

        let best_consumer = self.find_highest_power_consumer().await?;
        info!("Best consumer: {:#?}", best_consumer);
        if best_consumer.is_none() {
            // No new consumer available
            return Ok(());
        }
        let best_consumer = best_consumer.unwrap();

        self.connect_new_consumer(state, best_consumer).await
    }

    pub async fn process_request(&self) -> Result<(), Error> {
        let request = self.context.wait_request().await;
        let device = self.context.get_device(request.id).await?;

        match request.data {
            policy::RequestData::NotifyAttached => {
                info!("Received notify attached from device {}", device.id().0);
                self.process_notify_attach().await
            }
            policy::RequestData::NotifyDetached => {
                info!("Received notify detached from device {}", device.id().0);
                self.process_notify_detach().await
            }
            policy::RequestData::NotifyConsumerCapability(capability) => {
                info!(
                    "Received notify consumer capability from device {}: {:#?}",
                    device.id().0,
                    capability
                );
                self.process_notify_consumer_power_capability().await
            }
            policy::RequestData::RequestProviderCapability(capability) => {
                info!(
                    "Received request provider capability from device {}: {:#?}",
                    device.id().0,
                    capability
                );
                self.process_request_provider_power_capabilities().await
            }
            policy::RequestData::NotifyDisconnect => {
                info!("Received notify disconnect from device {}", device.id().0);
                self.process_notify_disconnect().await
            }
        }
    }
}

impl comms::MailboxDelegate for PowerPolicy {
    fn receive(&self, _message: &comms::Message) {}
}

#[embassy_executor::task]
pub async fn task() {
    info!("Starting power policy task");
    static POLICY: OnceLock<PowerPolicy> = OnceLock::new();
    let policy = POLICY.get_or_init(|| PowerPolicy::create().expect("Power policy singleton already initialized"));

    if comms::register_endpoint(policy, &policy.tp).await.is_err() {
        error!("Failed to register power policy endpoint");
        return;
    }

    loop {
        if let Err(e) = policy.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
