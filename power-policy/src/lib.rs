#![no_std]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::power::policy::device::{state_machine as sm, Device};
use embedded_services::power::policy::{policy, *};
use embedded_services::{comms, error, info};

/// State of the current sink
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SinkState {
    /// The ID of the currently connected sink
    device_id: DeviceId,
    /// The power capability of the currently connected sink
    power_capability: PowerCapability,
}

impl PartialOrd for SinkState {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.power_capability.cmp(&other.power_capability))
    }
}

impl Ord for SinkState {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.power_capability.cmp(&other.power_capability)
    }
}

struct InternalState {
    /// Current sink state, if any
    current_sink_state: Option<SinkState>,
}

impl InternalState {
    fn new() -> Self {
        Self {
            current_sink_state: None,
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

    async fn process_notify_attach(&self, device: &Device) -> Result<(), Error> {
        info!("Device {} received attach", device.id().0);
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_detach(&self, device: &Device) -> Result<(), Error> {
        info!("Device {} received detach", device.id().0);
        self.select_sink().await?;
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_sink_power_capability(
        &self,
        device: &Device,
        capability: Option<PowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "Device {} received sink power capability {:#?}",
            device.id().0,
            capability
        );

        self.select_sink().await?;
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_request_source_power_capabilities(
        &self,
        device: &Device,
        capability: PowerCapability,
    ) -> Result<(), Error> {
        info!(
            "Device {} requested source power capability {:#?}",
            device.id().0,
            capability
        );
        self.context.send_response(Ok(policy::ResponseData::Complete)).await;
        Ok(())
    }

    async fn process_notify_disconnect(&self, device: &Device) -> Result<(), Error> {
        info!("Device {} received disconnect", device.id().0);
        self.select_sink().await?;
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

    /// Determines and connects the best sink
    async fn select_sink(&self) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        info!("Selecting sink, current sink: {:#?}", state.current_sink_state);
        let mut best_sink = None;

        for node in self.context.devices().await {
            let device = node.data::<Device>().ok_or(Error::InvalidDevice)?;

            if let Some(current_sink) = state.current_sink_state {
                if device.id() == current_sink.device_id && !device.is_sink().await {
                    // Our sink has disconnected, don't consider this device
                    info!("Device {}, current sink disconnected", device.id().0);
                    continue;
                }
            }

            best_sink = match (best_sink, device.sink_capability().await) {
                // Nothing available
                (None, None) => None,
                // No existing sink
                (None, Some(power_capability)) => Some(SinkState {
                    device_id: device.id(),
                    power_capability,
                }),
                // Existing sink, no new sink
                (Some(_), None) => best_sink,
                // Existing sink, new sink
                (Some(best), Some(available)) => {
                    if available > best.power_capability {
                        Some(SinkState {
                            device_id: device.id(),
                            power_capability: available,
                        })
                    } else {
                        best_sink
                    }
                }
            };
        }

        info!("Best sink: {:#?}", best_sink);
        if let Some(best_sink) = best_sink {
            // See if we need to disconnect the current sink
            if let Some(current_sink) = state.current_sink_state {
                if let Ok(sink) = self
                    .context
                    .get_device(current_sink.device_id)
                    .await?
                    .try_policy_state_machine::<sm::Sink>()
                    .await
                {
                    if best_sink.device_id == current_sink.device_id
                        && best_sink.power_capability == current_sink.power_capability
                    {
                        // If the sink is the same device, capability and is still available, we don't need to do anything
                        info!("Best sink is the same, not switching");
                        return Ok(());
                    }

                    self.comms_notify(CommsMessage {
                        data: CommsData::SinkDisconnected(current_sink.device_id),
                    })
                    .await;
                    // Disconnect the current sink
                    info!("Device {}, disconnecting current sink", current_sink.device_id.0);
                    sink.disconnect().await?;
                }
            }

            info!("Device {}, connecting new sink", best_sink.device_id.0);
            return match self
                .context
                .get_device(best_sink.device_id)
                .await?
                .try_policy_state_machine::<sm::Attached>()
                .await
            {
                Ok(attached) => {
                    attached.connect_sink(best_sink.power_capability).await?;
                    state.current_sink_state = Some(best_sink);
                    self.comms_notify(CommsMessage {
                        data: CommsData::SinkConnected(best_sink.device_id, best_sink.power_capability),
                    })
                    .await;
                    Ok(())
                }
                Err(e) => {
                    error!("Error connecting sink: {:#?}", e);
                    Err(e)
                }
            };
        }

        Ok(())
    }

    pub async fn process_request(&self) -> Result<(), Error> {
        let request = self.context.wait_request().await;
        let device = self.context.get_device(request.id).await?;

        match request.data {
            policy::RequestData::NotifyAttached => self.process_notify_attach(device).await,
            policy::RequestData::NotifyDetached => self.process_notify_detach(device).await,
            policy::RequestData::NotifySinkCapability(capability) => {
                self.process_notify_sink_power_capability(device, capability).await
            }
            policy::RequestData::RequestSourceCapability(capability) => {
                self.process_request_source_power_capabilities(device, capability).await
            }
            policy::RequestData::NotifyDisconnect => self.process_notify_disconnect(device).await,
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
