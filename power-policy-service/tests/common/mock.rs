use std::collections::VecDeque;

use embassy_sync::mutex::Mutex;
use embedded_services::{
    GlobalRawMutex, info,
    power::policy::{self, device},
};

pub struct Mock {
    pub device: policy::device::Device,
    pub messages: Mutex<GlobalRawMutex, VecDeque<device::CommandData>>,
}

impl Mock {
    pub fn new(id: policy::DeviceId) -> Self {
        Self {
            device: policy::device::Device::new(id),
            messages: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn process_request(&self) -> Result<(), policy::Error> {
        let request = self.device.receive().await;
        match request.command {
            device::CommandData::ConnectAsConsumer(capability) => {
                info!(
                    "Device {} received connect consumer at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::CommandData::ConnectAsProvider(capability) => {
                info!(
                    "Device {} received connect provider at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::CommandData::Disconnect => {
                info!("Device {} received disconnect", self.device.id().0);
            }
        }
        self.messages.lock().await.push_back(request.command);
        request.respond(Ok(policy::device::ResponseData::Complete));
        Ok(())
    }
}

impl policy::device::DeviceContainer for Mock {
    fn get_power_policy_device(&self) -> &policy::device::Device {
        &self.device
    }
}
