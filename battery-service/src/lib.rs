#![no_std]

use context::BatteryEvent;
use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};

pub mod context;
pub mod controller;
pub mod device;
pub mod wrapper;

/// Standard Battery Service.
pub struct Service {
    pub endpoint: comms::Endpoint,
    pub context: context::Context,
}

impl Service {
    /// Create a new battery service instance.
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            context: context::Context::default(),
        }
    }

    /// Main battery service processing function.
    pub async fn process(&self) {
        let event = self.context.wait_event().await;
        self.context.process(event).await;
    }
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(event) = message.data.get::<BatteryEvent>() {
            self.context.send_event_no_wait(*event).map_err(|e| match e {
                embassy_sync::channel::TrySendError::Full(_) => comms::MailboxDelegateError::BufferFull,
            })?
        }

        Ok(())
    }
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Battery service task.
#[embassy_executor::task]
pub async fn task() {
    info!("Starting battery-service task");

    let service = SERVICE.get_or_init(Service::default);

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register battery service endpoint");
        return;
    }

    loop {
        service.process().await;
    }
}
