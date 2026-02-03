//! Thermal service context
use crate::{Error, Event, fan, sensor};
use embassy_sync::channel::Channel;
use embedded_services::GlobalRawMutex;
use embedded_services::{error, intrusive_list};

pub(crate) struct Context {
    // Registered temperature sensors
    sensors: intrusive_list::IntrusiveList,
    // Registered fans
    fans: intrusive_list::IntrusiveList,
    // Pending MPTF request queue
    mptf: Channel<GlobalRawMutex, thermal_service_messages::ThermalRequest, 10>,
    // Event queue
    events: Channel<GlobalRawMutex, Event, 10>,
}

impl Context {
    pub(crate) const fn new() -> Self {
        Self {
            sensors: intrusive_list::IntrusiveList::new(),
            fans: intrusive_list::IntrusiveList::new(),
            mptf: Channel::new(),
            events: Channel::new(),
        }
    }

    pub(crate) fn register_sensor(&self, sensor: &'static sensor::Device) -> Result<(), intrusive_list::Error> {
        if self.get_sensor(sensor.id()).is_some() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.sensors.push(sensor)
    }

    pub(crate) fn sensors(&self) -> &intrusive_list::IntrusiveList {
        &self.sensors
    }

    pub(crate) fn get_sensor(&self, id: sensor::DeviceId) -> Option<&'static sensor::Device> {
        for sensor in &self.sensors {
            if let Some(data) = sensor.data::<sensor::Device>() {
                if data.id() == id {
                    return Some(data);
                }
            } else {
                error!("Non-device located in sensors list");
            }
        }

        None
    }

    pub(crate) async fn execute_sensor_request(
        &self,
        id: sensor::DeviceId,
        request: sensor::Request,
    ) -> sensor::Response {
        let sensor = self.get_sensor(id).ok_or(sensor::Error::InvalidRequest)?;
        sensor.execute_request(request).await
    }

    pub(crate) fn register_fan(&self, fan: &'static fan::Device) -> Result<(), intrusive_list::Error> {
        if self.get_fan(fan.id()).is_some() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.fans.push(fan)
    }

    pub(crate) fn fans(&self) -> &intrusive_list::IntrusiveList {
        &self.fans
    }

    pub(crate) fn get_fan(&self, id: fan::DeviceId) -> Option<&'static fan::Device> {
        for fan in &self.fans {
            if let Some(data) = fan.data::<fan::Device>() {
                if data.id() == id {
                    return Some(data);
                }
            } else {
                error!("Non-device located in fan list");
            }
        }

        None
    }

    pub(crate) async fn execute_fan_request(&self, id: fan::DeviceId, request: fan::Request) -> fan::Response {
        let fan = self.get_fan(id).ok_or(fan::Error::InvalidRequest)?;
        fan.execute_request(request).await
    }

    pub(crate) fn queue_mptf_request(&self, msg: thermal_service_messages::ThermalRequest) -> Result<(), Error> {
        self.mptf.try_send(msg).map_err(|_| Error)
    }

    pub(crate) async fn wait_mptf_request(&self) -> thermal_service_messages::ThermalRequest {
        self.mptf.receive().await
    }

    pub(crate) async fn send_event(&self, event: Event) {
        self.events.send(event).await
    }

    pub(crate) async fn wait_event(&self) -> Event {
        self.events.receive().await
    }
}
