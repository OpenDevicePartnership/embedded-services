//! Thermal service
#![no_std]
#![allow(clippy::todo)]
#![allow(clippy::unwrap_used)]

use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::{comms, error, info, intrusive_list};

mod context;
pub mod fan;
#[cfg(feature = "mock")]
pub mod mock;
pub mod mptf;
pub mod sensor;
pub mod task;
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

pub struct Service {
    context: context::Context,
    endpoint: comms::Endpoint,
}

impl Service {
    pub async fn new(
        service_storage: &'static embassy_sync::once_lock::OnceLock<Service>,
        sensors: &[&'static sensor::Device],
        fans: &[&'static fan::Device],
    ) -> Result<&'static Self, Error> {
        let service = service_storage.get_or_init(|| Self {
            context: context::Context::new(),
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Thermal)),
        });

        for sensor in sensors {
            service.register_sensor(sensor).unwrap();
        }

        for fan in fans {
            service.register_fan(fan).unwrap();
        }

        service.init().await?;

        Ok(service)
    }

    async fn init(&'static self) -> Result<(), Error> {
        info!("Starting thermal service task");

        if comms::register_endpoint(self, &self.endpoint).await.is_err() {
            error!("Failed to register thermal service endpoint");
            Err(Error)
        } else {
            Ok(())
        }
    }

    /// Used to send messages to other services from the Thermal service,
    /// such as notifying the Host of thresholds crossed or the Power service if CRT TEMP is reached.
    pub async fn send_service_msg(
        &self,
        to: comms::EndpointID,
        data: &(impl embedded_services::Any + Send + Sync),
    ) -> Result<(), Error> {
        // TODO: When this gets updated to return error, handle retrying send on failure
        self.endpoint.send(to, data).await.map_err(|_| Error)?;
        Ok(())
    }

    /// Send a MPTF request
    pub fn queue_mptf_request(&self, msg: thermal_service_messages::ThermalRequest) -> Result<(), Error> {
        self.context.queue_mptf_request(msg)
    }

    /// Wait for a MPTF request
    pub async fn wait_mptf_request(&self) -> thermal_service_messages::ThermalRequest {
        self.context.wait_mptf_request().await
    }

    /// Send a thermal event
    pub async fn send_event(&self, event: Event) {
        self.context.send_event(event).await
    }

    /// Wait for a thermal event
    pub async fn wait_event(&self) -> Event {
        self.context.wait_event().await
    }

    /// Register a sensor with the thermal service
    pub fn register_sensor(&self, sensor: &'static sensor::Device) -> Result<(), intrusive_list::Error> {
        self.context.register_sensor(sensor)
    }

    /// Provides access to the sensors list
    pub fn sensors(&'static self) -> &'static intrusive_list::IntrusiveList {
        self.context.sensors()
    }

    /// Find a sensor by its ID
    pub fn get_sensor(&self, id: sensor::DeviceId) -> Option<&'static sensor::Device> {
        self.context.get_sensor(id)
    }

    /// Send a request to a sensor through the thermal service instead of directly.
    pub async fn execute_sensor_request(&self, id: sensor::DeviceId, request: sensor::Request) -> sensor::Response {
        self.context.execute_sensor_request(id, request).await
    }

    /// Register a fan with the thermal service
    pub fn register_fan(&self, fan: &'static fan::Device) -> Result<(), intrusive_list::Error> {
        self.context.register_fan(fan)
    }

    /// Provides access to the fans list
    pub fn fans(&'static self) -> &'static intrusive_list::IntrusiveList {
        self.context.fans()
    }

    /// Find a fan by its ID
    pub fn get_fan(&self, id: fan::DeviceId) -> Option<&'static fan::Device> {
        self.context.get_fan(id)
    }

    /// Send a request to a fan through the thermal service instead of directly.
    pub async fn execute_fan_request(&self, id: fan::DeviceId, request: fan::Request) -> fan::Response {
        self.context.execute_fan_request(id, request).await
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // Queue for later processing
        if let Some(msg) = message.data.get::<thermal_service_messages::ThermalRequest>() {
            self.context
                .queue_mptf_request(*msg)
                .map_err(|_| comms::MailboxDelegateError::BufferFull)
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}
