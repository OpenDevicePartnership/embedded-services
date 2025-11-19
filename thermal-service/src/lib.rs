//! Thermal service
#![no_std]

use embassy_sync::once_lock::OnceLock;
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::buffer::OwnedRef;
use embedded_services::ec_type::message::StdHostRequest;
use embedded_services::{comms, error, info, intrusive_list};

mod context;
pub mod fan;
pub mod mptf;
pub mod sensor;
pub mod utils;

/// Thermal error
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Error;

/// Thermal event
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    /// Sensor sampled temperature exceeding a threshold
    ThresholdExceeded(sensor::DeviceId, sensor::ThresholdType, DegreesCelsius),
    /// Sensor is no longer exceeding a threshold
    ThresholdCleared(sensor::DeviceId, sensor::ThresholdType),
    /// Sensor encountered hardware failure
    SensorFailure(sensor::DeviceId, sensor::Error),
    /// Fan encountered hardware failure
    FanFailure(fan::DeviceId, fan::Error),
}

struct Service<'a> {
    context: context::Context<'a>,
    endpoint: comms::Endpoint,
}

impl<'a> Service<'a> {
    fn new() -> Self {
        Self {
            context: context::Context::new(),
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Thermal)),
        }
    }
}

impl<'a> comms::MailboxDelegate for Service<'a> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Queue for later processing
        if let Some(msg) = message.data.get::<StdHostRequest>() {
            self.context
                .send_mctp_payload(*msg)
                .map_err(|_| comms::MailboxDelegateError::BufferFull)
        } else if let Some(&msg) = message.data.get::<mptf::Request>() {
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
pub async fn init() -> Result<(), Error> {
    info!("Starting thermal service task");
    let service = SERVICE.get_or_init(Service::new);

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register thermal service endpoint");
        Err(Error)
    } else {
        Ok(())
    }
}

// TODO: Don't like the code duplication from all these wrappers, consider better approach

/// Used to send messages to other services from the Thermal service,
/// such as notifying the Host of thresholds crossed or the Power service if CRT TEMP is reached.
pub async fn send_service_msg(to: comms::EndpointID, data: &impl embedded_services::Any) -> Result<(), Error> {
    // TODO: When this gets updated to return error, handle retrying send on failure
    SERVICE.get().await.endpoint.send(to, data).await.map_err(|_| Error)?;
    Ok(())
}

/// Send a MPTF request
pub async fn queue_mptf_request(msg: mptf::Request) -> Result<(), Error> {
    SERVICE.get().await.context.send_mptf_request(msg)
}

/// Wait for a MPTF request
pub async fn wait_mptf_request() -> mptf::Request {
    SERVICE.get().await.context.wait_mptf_request().await
}

/// Wait for a MCTP payload
pub async fn wait_mctp_payload() -> StdHostRequest {
    SERVICE.get().await.context.wait_mctp_payload().await
}

pub fn get_mctp_buf<'a>() -> &'a OwnedRef<'a, u8> {
    SERVICE.try_get().unwrap().context.get_mctp_buf()
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
    SERVICE.get().await.context.register_sensor(sensor)
}

/// Provides access to the sensors list
pub async fn sensors() -> &'static intrusive_list::IntrusiveList {
    SERVICE.get().await.context.sensors()
}

/// Find a sensor by its ID
pub async fn get_sensor(id: sensor::DeviceId) -> Option<&'static sensor::Device> {
    SERVICE.get().await.context.get_sensor(id)
}

/// Send a request to a sensor through the thermal service instead of directly.
pub async fn execute_sensor_request(id: sensor::DeviceId, request: sensor::Request) -> sensor::Response {
    SERVICE.get().await.context.execute_sensor_request(id, request).await
}

/// Register a fan with the thermal service
pub async fn register_fan(fan: &'static fan::Device) -> Result<(), intrusive_list::Error> {
    SERVICE.get().await.context.register_fan(fan)
}

/// Provides access to the fans list
pub async fn fans() -> &'static intrusive_list::IntrusiveList {
    SERVICE.get().await.context.fans()
}

/// Find a fan by its ID
pub async fn get_fan(id: fan::DeviceId) -> Option<&'static fan::Device> {
    SERVICE.get().await.context.get_fan(id)
}

/// Send a request to a fan through the thermal service instead of directly.
pub async fn execute_fan_request(id: fan::DeviceId, request: fan::Request) -> fan::Response {
    SERVICE.get().await.context.execute_fan_request(id, request).await
}
