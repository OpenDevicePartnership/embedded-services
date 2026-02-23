//! Thermal service
#![no_std]
#![allow(clippy::todo)]
#![allow(clippy::unwrap_used)]

use embedded_sensors_hal_async::temperature::DegreesCelsius;
use thermal_service_messages::{ThermalRequest, ThermalResult};

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

pub struct Service<'hw> {
    context: context::Context<'hw>,
}

impl<'hw> Service<'hw> {
    pub async fn init(
        service_storage: &'hw embassy_sync::once_lock::OnceLock<Service<'hw>>,
        sensors: &'hw [&'hw sensor::Device],
        fans: &'hw [&'hw fan::Device],
    ) -> &'hw Self {
        service_storage.get_or_init(|| Self {
            context: context::Context::new(sensors, fans),
        })
    }

    /// Send a thermal event
    pub async fn send_event(&self, event: Event) {
        self.context.send_event(event).await
    }

    /// Wait for a thermal event
    pub async fn wait_event(&self) -> Event {
        self.context.wait_event().await
    }

    /// Provides access to the sensors list
    pub fn sensors(&self) -> &[&sensor::Device] {
        self.context.sensors()
    }

    /// Find a sensor by its ID
    pub fn get_sensor(&self, id: sensor::DeviceId) -> Option<&sensor::Device> {
        self.context.get_sensor(id)
    }

    /// Send a request to a sensor through the thermal service instead of directly.
    pub async fn execute_sensor_request(&self, id: sensor::DeviceId, request: sensor::Request) -> sensor::Response {
        self.context.execute_sensor_request(id, request).await
    }

    /// Provides access to the fans list
    pub fn fans(&self) -> &[&fan::Device] {
        self.context.fans()
    }

    /// Find a fan by its ID
    pub fn get_fan(&self, id: fan::DeviceId) -> Option<&fan::Device> {
        self.context.get_fan(id)
    }

    /// Send a request to a fan through the thermal service instead of directly.
    pub async fn execute_fan_request(&self, id: fan::DeviceId, request: fan::Request) -> fan::Response {
        self.context.execute_fan_request(id, request).await
    }
}

impl<'hw> embedded_services::relay::mctp::RelayServiceHandlerTypes for Service<'hw> {
    type RequestType = ThermalRequest;
    type ResultType = ThermalResult;
}

impl<'hw> embedded_services::relay::mctp::RelayServiceHandler for Service<'hw> {
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        mptf::process_request(&request, self).await
    }
}
