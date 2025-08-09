#![no_std]

use core::{any::Any, convert::Infallible};

use context::BatteryEvent;
use embassy_futures::select::select;
use embassy_sync::once_lock::OnceLock;
use embedded_services::{
    comms::{self, EndpointID},
    ec_type::message::AcpiMsgComms,
    error, info, trace,
};

mod acpi;
pub mod context;
pub mod controller;
pub mod device;
pub mod wrapper;

/// Standard Battery Service.
pub struct Service<'a> {
    pub endpoint: comms::Endpoint,
    pub context: context::Context<'a>,
}

impl<'a> Service<'a> {
    /// Create a new battery service instance.
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            context: context::Context::default(),
        }
    }

    /// Create a new battery service instance with context configuration.
    pub fn new_with_ctx_config(config: context::Config) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            context: context::Context::new_with_config(config),
        }
    }

    /// Main battery service processing function.
    pub async fn process_next(&self) {
        let event = self.wait_next().await;
        self.process_event(event).await
    }

    /// Wait for next event.
    pub async fn wait_next(&self) -> Event<'a> {
        match select(self.context.wait_event(), self.context.wait_acpi_cmd()).await {
            embassy_futures::select::Either::First(event) => Event::StateMachine(event),
            embassy_futures::select::Either::Second(acpi_msg) => Event::AcpiRequest(acpi_msg),
        }
    }

    /// Process battery service event.
    pub async fn process_event(&self, event: Event<'a>) {
        match event {
            Event::StateMachine(event) => {
                trace!("Battery service: state machine event recvd {:?}", event);
                self.context.process(event).await
            }
            Event::AcpiRequest(acpi_msg) => {
                trace!("Battery service: ACPI cmd recvd");
                self.context.process_acpi_cmd(acpi_msg).await
            }
        }
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event<'a> {
    StateMachine(BatteryEvent),
    AcpiRequest(AcpiMsgComms<'a>),
}

impl<'a> Default for Service<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> comms::MailboxDelegate for Service<'a> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(event) = message.data.get::<BatteryEvent>() {
            self.context.send_event_no_wait(*event).map_err(|e| match e {
                embassy_sync::channel::TrySendError::Full(_) => comms::MailboxDelegateError::BufferFull,
            })?
        } else if let Some(acpi_cmd) = message.data.get::<AcpiMsgComms>() {
            self.context.send_acpi_cmd(acpi_cmd.clone());
        } else if let Some(power_policy_msg) = message.data.get::<embedded_services::power::policy::CommsMessage>() {
            self.context.set_power_info(&power_policy_msg.data)?;
        }

        Ok(())
    }
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Register fuel gauge device with the battery service.
///
/// Must be done before sending the battery service commands so that hardware device is visible
/// to the battery service.
pub async fn register_fuel_gauge(
    device: &'static device::Device,
) -> Result<(), embedded_services::intrusive_list::Error> {
    let service = SERVICE.get().await;

    service.context.register_fuel_gauge(device).await?;

    Ok(())
}

/// Use the battery service endpoint to send data to other subsystems and services.
pub async fn comms_send(endpoint_id: EndpointID, data: &impl Any) -> Result<(), Infallible> {
    let service = SERVICE.get().await;

    service.endpoint.send(endpoint_id, data).await
}

/// Send the battery service state machine an event and await a response.
///
/// This is an alternative method of interacting with the battery service (instead of using the comms service),
/// and is a useful fn if you want to send an event and await a response sequentially.
pub async fn execute_event(event: BatteryEvent) -> context::BatteryResponse {
    let service = SERVICE.get().await;

    service.context.execute_event(event).await
}

/// Wait for a response from the battery service.
///
/// Use this function after sending the battery service a message via the comms system.
pub async fn wait_for_battery_response() -> context::BatteryResponse {
    let service = SERVICE.get().await;

    service.context.wait_response().await
}

/// Asynchronously query the state from the state machine.
pub async fn get_state() -> context::State {
    let service = SERVICE.get().await;

    service.context.get_state().await
}

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
        service.process_next().await;
    }
}
