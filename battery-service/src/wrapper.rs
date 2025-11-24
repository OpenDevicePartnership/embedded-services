use embassy_futures::select::select;
use embassy_sync::mutex::Mutex;
use embedded_services::GlobalRawMutex;
use embedded_services::trace;

use crate::{
    controller::{Controller, ControllerEvent},
    device::{Command, Device},
};

/// Wrapper object to bind device to fuel gauge hardware driver.
pub struct Wrapper<'a, C: Controller> {
    device: &'a Device,
    controller: Mutex<GlobalRawMutex, C>,
}

impl<'a, C: Controller> Wrapper<'a, C> {
    /// Create a new fuel gauge wrapper.
    pub fn new(device: &'a Device, controller: C) -> Self {
        // Set device timeout when constructing.
        device.set_timeout(controller.get_timeout());

        Self {
            device,
            controller: Mutex::new(controller),
        }
    }

    /// Process events from hardware controller or context device.
    /// Only call this fn ONCE, it will infinitely loop processing messages. Otherwise a deadlock could occur.
    pub async fn process(&self) {
        let mut controller = self.controller.lock().await;
        loop {
            let res = select(controller.get_device_event(), self.device.receive_command()).await;
            match res {
                embassy_futures::select::Either::First(event) => {
                    trace!("New fuel gauge hardware device event.");
                    self.process_device_event(&mut controller, self.device, event);
                }
                embassy_futures::select::Either::Second(cmd) => {
                    trace!("New fuel gauge state machine command.");
                    self.process_context_command(&mut controller, self.device, cmd).await;
                }
            };
        }
    }

    fn process_device_event(&self, _controller: &mut C, _device: &Device, event: ControllerEvent) {
        // TODO: add events
        match event {}
    }

    async fn process_context_command(&self, controller: &mut C, device: &Device, command: Command) {
        match command {
            Command::Initialize => match controller.initialize().await {
                Ok(_) => {
                    device
                        .send_response(Ok(crate::device::InternalResponse::Complete))
                        .await;
                }
                Err(_e) => {
                    // TODO: Add specific error handling
                    device.send_response(Err(crate::device::FuelGaugeError::BusError)).await;
                }
            },
            Command::Ping => match controller.ping().await {
                Ok(_) => {
                    device
                        .send_response(Ok(crate::device::InternalResponse::Complete))
                        .await;
                }
                Err(_e) => {
                    // TODO: Add specific error handling
                    device.send_response(Err(crate::device::FuelGaugeError::BusError)).await;
                }
            },
            Command::UpdateStaticCache => match controller.get_static_data().await {
                Ok(static_data) => {
                    device.set_static_battery_cache(static_data).await;
                    device
                        .send_response(Ok(crate::device::InternalResponse::Complete))
                        .await;
                }
                Err(_e) => {
                    // TODO: Add specific error handling
                    device.send_response(Err(crate::device::FuelGaugeError::BusError)).await;
                }
            },
            Command::UpdateDynamicCache => match controller.get_dynamic_data().await {
                Ok(dynamic_data) => {
                    device.set_dynamic_battery_cache(dynamic_data).await;
                    device
                        .send_response(Ok(crate::device::InternalResponse::Complete))
                        .await;
                }
                Err(_e) => {
                    // TODO: Add specific error handling
                    device.send_response(Err(crate::device::FuelGaugeError::BusError)).await;
                }
            },
        }
    }
}
