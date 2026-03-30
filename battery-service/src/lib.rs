#![no_std]

use core::{any::Any, convert::Infallible};

use battery_service_messages::{AcpiBatteryError, AcpiBatteryRequest, AcpiBatteryResult};
use context::BatteryEvent;
use embedded_services::{
    comms::{self, EndpointID},
    error, info, trace,
};

mod acpi;
pub mod context;
pub mod controller;
pub mod device;
#[cfg(feature = "mock")]
pub mod mock;
pub mod wrapper;

/// Parameters required to initialize the battery service.
pub struct InitParams<'hw, const N: usize> {
    pub devices: [&'hw device::Device; N],
    pub config: context::Config,
}

/// The main service implementation.
struct ServiceInner<const N: usize> {
    endpoint: comms::Endpoint,
    context: context::Context,
}

impl<const N: usize> ServiceInner<N> {
    fn new(config: context::Config) -> Self {
        Self {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            context: context::Context::new_with_config(config),
        }
    }

    /// Main battery service processing function.
    async fn process_next(&self) {
        let event = self.wait_next().await;
        self.process_event(event).await
    }

    /// Wait for next event.
    async fn wait_next(&self) -> BatteryEvent {
        self.context.wait_event().await
    }

    /// Process battery service event.
    async fn process_event(&self, event: BatteryEvent) {
        trace!("Battery service: state machine event recvd {:?}", event);
        self.context.process(event).await
    }

    /// Register fuel gauge device with the battery service.
    fn register_fuel_gauge(
        &self,
        device: &'static device::Device,
    ) -> Result<(), embedded_services::intrusive_list::Error> {
        self.context.register_fuel_gauge(device)?;
        Ok(())
    }

    /// Use the battery service endpoint to send data to other subsystems and services.
    async fn comms_send(&self, endpoint_id: EndpointID, data: &(impl Any + Send + Sync)) -> Result<(), Infallible> {
        self.endpoint.send(endpoint_id, data).await
    }

    /// Send the battery service state machine an event and await a response.
    async fn execute_event(&self, event: BatteryEvent) -> context::BatteryResponse {
        self.context.execute_event(event).await
    }

    /// Wait for a response from the battery service.
    async fn wait_for_battery_response(&self) -> context::BatteryResponse {
        self.context.wait_response().await
    }

    /// Asynchronously query the state from the state machine.
    async fn get_state(&self) -> context::State {
        self.context.get_state().await
    }
}

/// The memory resources required by the battery service.
#[derive(Default)]
pub struct Resources<const N: usize> {
    inner: Option<ServiceInner<N>>,
}

/// A task runner for the battery service. Users of the service must run this object in an embassy task or similar async execution context.
pub struct Runner<'hw, const N: usize> {
    service: &'hw ServiceInner<N>,
}

impl<'hw, const N: usize> odp_service_common::runnable_service::ServiceRunner<'hw> for Runner<'hw, N> {
    /// Run the service.
    async fn run(self) -> embedded_services::Never {
        info!("Starting battery-service");
        loop {
            self.service.process_next().await;
        }
    }
}

/// Control handle for the battery service. Use this to interact with the battery service.
#[derive(Clone, Copy)]
pub struct Service<'hw, const N: usize> {
    inner: &'hw ServiceInner<N>,
}

impl<'hw, const N: usize> Service<'hw, N> {
    /// Main battery service processing function.
    pub async fn process_next(&self) {
        self.inner.process_next().await
    }

    /// Wait for next event.
    pub async fn wait_next(&self) -> BatteryEvent {
        self.inner.wait_next().await
    }

    /// Process battery service event.
    pub async fn process_event(&self, event: BatteryEvent) {
        self.inner.process_event(event).await
    }

    /// Use the battery service endpoint to send data to other subsystems and services.
    pub async fn comms_send(&self, endpoint_id: EndpointID, data: &(impl Any + Send + Sync)) -> Result<(), Infallible> {
        self.inner.comms_send(endpoint_id, data).await
    }

    /// Send the battery service state machine an event and await a response.
    ///
    /// This is an alternative method of interacting with the battery service (instead of using the comms service),
    /// and is a useful fn if you want to send an event and await a response sequentially.
    pub async fn execute_event(&self, event: BatteryEvent) -> context::BatteryResponse {
        self.inner.execute_event(event).await
    }

    /// Wait for a response from the battery service.
    ///
    /// Use this function after sending the battery service a message via the comms system.
    pub async fn wait_for_battery_response(&self) -> context::BatteryResponse {
        self.inner.wait_for_battery_response().await
    }

    /// Asynchronously query the state from the state machine.
    pub async fn get_state(&self) -> context::State {
        self.inner.get_state().await
    }
}

/// Errors that can occur during battery service initialization.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitError {
    DeviceRegistrationFailed(crate::device::DeviceId),
    CommsRegistrationFailed,
}

impl<'hw, const N: usize> odp_service_common::runnable_service::Service<'hw> for Service<'hw, N>
where
    'hw: 'static, // TODO relax this 'static requirement when we drop usages of IntrusiveList (including comms)
{
    type Runner = Runner<'hw, N>;
    type ErrorType = InitError;
    type InitParams = InitParams<'hw, N>;
    type Resources = Resources<N>;

    async fn new(
        service_storage: &'hw mut Resources<N>,
        init_params: Self::InitParams,
    ) -> Result<(Self, Runner<'hw, N>), InitError> {
        let service = service_storage.inner.insert(ServiceInner::new(init_params.config));

        for device in init_params.devices {
            if service.register_fuel_gauge(device).is_err() {
                error!("Failed to register battery device with DeviceId {:?}", device.id());
                return Err(InitError::DeviceRegistrationFailed(device.id()));
            }
        }

        if comms::register_endpoint(service, &service.endpoint).await.is_err() {
            error!("Failed to register battery service endpoint");
            return Err(InitError::CommsRegistrationFailed);
        }

        Ok((Self { inner: service }, Runner { service }))
    }
}

impl<const N: usize> embedded_services::relay::mctp::RelayServiceHandlerTypes for Service<'_, N> {
    type RequestType = AcpiBatteryRequest;
    type ResultType = AcpiBatteryResult;
}

impl<const N: usize> embedded_services::relay::mctp::RelayServiceHandler for Service<'_, N> {
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        trace!("Battery service: ACPI cmd recvd");
        let response = self.inner.context.process_acpi_cmd(&request).await;
        if let Err(e) = response {
            error!("Battery service command failed: {:?}", e)
        }
        response
    }
}

impl<const N: usize> comms::MailboxDelegate for ServiceInner<N> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(event) = message.data.get::<BatteryEvent>() {
            self.context.send_event_no_wait(*event).map_err(|e| match e {
                embassy_sync::channel::TrySendError::Full(_) => comms::MailboxDelegateError::BufferFull,
            })?
        }
        // TODO: Migrate away from using comms for power policy updates
        // See https://github.com/OpenDevicePartnership/embedded-services/issues/742
        /*else if let Some(power_policy_msg) = message
            .data
            .get::<power_policy_interface::service::event::CommsMessage>()
        {
            self.context.set_power_info(&power_policy_msg.data)?;
        }*/

        Ok(())
    }
}
