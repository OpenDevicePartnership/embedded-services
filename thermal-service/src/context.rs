//! Thermal service context
use crate::{Event, fan, sensor};
use embassy_sync::channel::Channel;
use embedded_services::GlobalRawMutex;

pub(crate) struct Context<'hw> {
    // Registered temperature sensors
    sensors: &'hw [&'hw sensor::Device],
    // Registered fans
    fans: &'hw [&'hw fan::Device],
    // Event queue
    events: Channel<GlobalRawMutex, Event, 10>,
}

impl<'hw> Context<'hw> {
    pub(crate) const fn new(sensors: &'hw [&'hw sensor::Device], fans: &'hw [&'hw fan::Device]) -> Self {
        Self {
            sensors,
            fans,
            events: Channel::new(),
        }
    }

    pub(crate) fn sensors(&self) -> &[&sensor::Device] {
        self.sensors
    }

    pub(crate) fn get_sensor(&self, id: sensor::DeviceId) -> Option<&sensor::Device> {
        self.sensors.iter().find(|sensor| sensor.id() == id).copied()
    }

    pub(crate) async fn execute_sensor_request(
        &self,
        id: sensor::DeviceId,
        request: sensor::Request,
    ) -> sensor::Response {
        let sensor = self.get_sensor(id).ok_or(sensor::Error::InvalidRequest)?;
        sensor.execute_request(request).await
    }

    pub(crate) fn fans(&self) -> &[&fan::Device] {
        self.fans
    }

    pub(crate) fn get_fan(&self, id: fan::DeviceId) -> Option<&fan::Device> {
        self.fans.iter().find(|fan| fan.id() == id).copied()
    }

    pub(crate) async fn execute_fan_request(&self, id: fan::DeviceId, request: fan::Request) -> fan::Response {
        let fan = self.get_fan(id).ok_or(fan::Error::InvalidRequest)?;
        fan.execute_request(request).await
    }

    pub(crate) async fn send_event(&self, event: Event) {
        self.events.send(event).await
    }

    pub(crate) async fn wait_event(&self) -> Event {
        self.events.receive().await
    }
}
