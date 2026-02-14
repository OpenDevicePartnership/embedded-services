#![no_std]

use core::{any::Any, convert::Infallible};

use battery_service_messages::{AcpiBatteryError, AcpiBatteryRequest};
use context::BatteryEvent;
use embassy_futures::select::select;
use embedded_services::{
    comms::{self, EndpointID},
    error, trace,
};

mod acpi;
pub mod context;
pub mod controller;
pub mod device;
#[cfg(feature = "mock")]
pub mod mock;
pub mod task;
pub mod wrapper;

/// Standard Battery Service.
pub struct Service {
    pub endpoint: comms::Endpoint,
    pub context: context::Context,
}

impl Service {
    /// Create a new battery service instance.
    pub const fn new() -> Self {
        Self::new_inner(context::Config::new())
    }

    /// Create a new battery service instance with context configuration.
    pub const fn new_with_ctx_config(config: context::Config) -> Self {
        Self::new_inner(config)
    }

    const fn new_inner(config: context::Config) -> Self {
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
    pub async fn wait_next(&self) -> Event {
        match select(self.context.wait_event(), self.context.wait_acpi_cmd()).await {
            embassy_futures::select::Either::First(event) => Event::StateMachine(event),
            embassy_futures::select::Either::Second(acpi_msg) => Event::AcpiRequest(acpi_msg),
        }
    }

    /// Process battery service event.
    pub async fn process_event(&self, event: Event) {
        match event {
            Event::StateMachine(event) => {
                trace!("Battery service: state machine event recvd {:?}", event);
                self.context.process(event).await
            }
            Event::AcpiRequest(acpi_msg) => {
                trace!("Battery service: ACPI cmd recvd");
                let response = self.context.process_acpi_cmd(&acpi_msg).await;
                if let Err(e) = response {
                    error!("Battery service command failed: {:?}", e)
                }

                // TODO We should probably be responding to the requestor rather than just assuming the request came from the host
                self.comms_send(
                    crate::EndpointID::External(embedded_services::comms::External::Host),
                    &response,
                )
                .await
                .expect("comms_send is infallible")
            }
        }
    }

    /// Register fuel gauge device with the battery service.
    ///
    /// Must be done before sending the battery service commands so that hardware device is visible
    /// to the battery service.
    pub(crate) fn register_fuel_gauge(
        &self,
        device: &'static device::Device,
    ) -> Result<(), embedded_services::intrusive_list::Error> {
        self.context.register_fuel_gauge(device)?;

        Ok(())
    }

    /// Use the battery service endpoint to send data to other subsystems and services.
    pub async fn comms_send(&self, endpoint_id: EndpointID, data: &(impl Any + Send + Sync)) -> Result<(), Infallible> {
        self.endpoint.send(endpoint_id, data).await
    }

    /// Send the battery service state machine an event and await a response.
    ///
    /// This is an alternative method of interacting with the battery service (instead of using the comms service),
    /// and is a useful fn if you want to send an event and await a response sequentially.
    pub async fn execute_event(&self, event: BatteryEvent) -> context::BatteryResponse {
        self.context.execute_event(event).await
    }

    /// Wait for a response from the battery service.
    ///
    /// Use this function after sending the battery service a message via the comms system.
    pub async fn wait_for_battery_response(&self) -> context::BatteryResponse {
        self.context.wait_response().await
    }

    /// Asynchronously query the state from the state machine.
    pub async fn get_state(&self) -> context::State {
        self.context.get_state().await
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    StateMachine(BatteryEvent),
    AcpiRequest(AcpiBatteryRequest),
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
        } else if let Some(battery_request) = message.data.get::<AcpiBatteryRequest>() {
            self.context.send_acpi_cmd(*battery_request);
        }
        // TODO: address in refactor
        /*else if let Some(power_policy_msg) = message.data.get::<power_policy_service::service::event::Event>()
        {
        self.context.set_power_info(&power_policy_msg.data)?;
        }*/

        Ok(())
    }
}
