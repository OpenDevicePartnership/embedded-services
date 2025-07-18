//! Thermal service
#![no_std]

use embassy_sync::once_lock::OnceLock;
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::{comms, error, info, intrusive_list};

mod context;
pub mod fan;
pub mod mptf;
pub mod sensor;
pub mod utils;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    ThresholdExceeded(sensor::DeviceId, sensor::ThresholdType, DegreesCelsius),
    ThresholdCleared(sensor::DeviceId, sensor::ThresholdType),
    SensorFailure(sensor::DeviceId, sensor::Error),
    FanFailure(fan::DeviceId, fan::Error),
}

struct Service {
    context: context::Context,
    endpoint: comms::Endpoint,
}

impl Service {
    fn new() -> Option<Self> {
        Some(Self {
            context: context::Context::new(),
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Thermal)),
        })
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Queue for later processing
        if let Some(&msg) = message.data.get::<mptf::Request>() {
            self.context
                .send_mptf_request(msg)
                .map_err(|_| comms::MailboxDelegateError::BufferFull)
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}

// Just one instance of the service should be running
static SERVICE: OnceLock<Service> = OnceLock::new();

/// This must be called to initialize the Thermal service
pub async fn init() {
    info!("Starting thermal service task");
    let service = SERVICE.get_or_init(|| Service::new().expect("Thermal service singleton already initialized"));

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register thermal service endpoint");
    }
}

// TODO: Don't like the code duplication from all these wrappers, consider better approach

/// Used to send messages to other services from the Thermal service,
/// such as notifying the Host of thresholds crossed or the Power service if CRT TEMP is reached.
pub async fn send_service_msg(to: comms::EndpointID, data: &impl embedded_services::Any) {
    // TODO: When this gets updated to return error, handle retrying send on failure
    SERVICE.get().await.endpoint.send(to, data).await.expect("Infallible");
}

/// Send a MPTF request
pub async fn queue_mptf_request(msg: mptf::Request) -> Result<(), ()> {
    SERVICE.get().await.context.send_mptf_request(msg)
}

/// Wait for a MPTF request
pub async fn wait_mptf_request() -> mptf::Request {
    SERVICE.get().await.context.wait_mptf_request().await
}

/// Send a thermal event
pub async fn send_event(event: Event) {
    SERVICE.get().await.context.send_event(event).await
}

/// Wait for a thermal event
pub async fn wait_event() -> Event {
    SERVICE.get().await.context.wait_event().await
}

/// Register a sensor with the thermal service
pub async fn register_sensor(sensor: &'static sensor::Device) -> Result<(), intrusive_list::Error> {
    SERVICE.get().await.context.register_sensor(sensor).await
}

/// Provides access to the sensors list
pub async fn sensors() -> &'static intrusive_list::IntrusiveList {
    SERVICE.get().await.context.sensors().await
}

/// Find a sensor by its ID
pub async fn get_sensor(id: sensor::DeviceId) -> Option<&'static sensor::Device> {
    SERVICE.get().await.context.get_sensor(id).await
}

/// Send a request to a sensor through the thermal service instead of directly.
pub async fn execute_sensor_request(id: sensor::DeviceId, request: sensor::Request) -> sensor::Response {
    SERVICE.get().await.context.execute_sensor_request(id, request).await
}

/// Register a fan with the thermal service
pub async fn register_fan(fan: &'static fan::Device) -> Result<(), intrusive_list::Error> {
    SERVICE.get().await.context.register_fan(fan).await
}

/// Provides access to the fans list
pub async fn fans() -> &'static intrusive_list::IntrusiveList {
    SERVICE.get().await.context.fans().await
}

/// Find a fan by its ID
pub async fn get_fan(id: fan::DeviceId) -> Option<&'static fan::Device> {
    SERVICE.get().await.context.get_fan(id).await
}

/// Send a request to a fan through the thermal service instead of directly.
pub async fn execute_fan_request(id: fan::DeviceId, request: fan::Request) -> fan::Response {
    SERVICE.get().await.context.execute_fan_request(id, request).await
}
