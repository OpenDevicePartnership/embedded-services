#![no_std]

use core::{any::Any, convert::Infallible};

use context::BatteryEvent;
use embedded_services::{
    comms::{self, EndpointID},
    error, info, trace,
};

use battery_service_interface::{
    BatteryError, Bct, BctReturnResult, BixFixedStrings, Bma, Bmc, Bmd, Bms, Bpc, Bps, Bpt, BstReturn, Btm,
    BtmReturnResult, Btp, DeviceId, PifFixedStrings, PsrReturn, StaReturn,
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

impl<'hw, const N: usize> battery_service_interface::BatteryService for Service<'hw, N> {
    async fn battery_charge_time(
        &self,
        battery_id: DeviceId,
        charge_level: Bct,
    ) -> Result<BctReturnResult, BatteryError> {
        self.inner.context.bct_handler(battery_id, charge_level).await
    }

    async fn battery_info(&self, battery_id: DeviceId) -> Result<BixFixedStrings, BatteryError> {
        self.inner.context.bix_handler(battery_id).await
    }

    async fn set_battery_measurement_averaging_interval(
        &self,
        battery_id: DeviceId,
        bma: Bma,
    ) -> Result<(), BatteryError> {
        self.inner.context.bma_handler(battery_id, bma).await
    }

    async fn battery_maintenance_control(&self, battery_id: DeviceId, bmc: Bmc) -> Result<(), BatteryError> {
        self.inner.context.bmc_handler(battery_id, bmc).await
    }

    async fn battery_maintenance_data(&self, battery_id: DeviceId) -> Result<Bmd, BatteryError> {
        self.inner.context.bmd_handler(battery_id).await
    }

    async fn set_battery_measurement_sampling_time(
        &self,
        battery_id: DeviceId,
        battery_measurement_sampling: Bms,
    ) -> Result<(), BatteryError> {
        self.inner
            .context
            .bms_handler(battery_id, battery_measurement_sampling)
            .await
    }

    async fn battery_power_characteristics(&self, battery_id: DeviceId) -> Result<Bpc, BatteryError> {
        self.inner.context.bpc_handler(battery_id).await
    }

    async fn battery_power_state(&self, battery_id: DeviceId) -> Result<Bps, BatteryError> {
        self.inner.context.bps_handler(battery_id).await
    }

    async fn set_battery_power_threshold(
        &self,
        battery_id: DeviceId,
        power_threshold: Bpt,
    ) -> Result<(), BatteryError> {
        self.inner.context.bpt_handler(battery_id, power_threshold).await
    }

    async fn battery_status(&self, battery_id: DeviceId) -> Result<BstReturn, BatteryError> {
        self.inner.context.bst_handler(battery_id).await
    }

    async fn battery_time_to_empty(
        &self,
        battery_id: DeviceId,
        battery_discharge_rate: Btm,
    ) -> Result<BtmReturnResult, BatteryError> {
        self.inner.context.btm_handler(battery_id, battery_discharge_rate).await
    }

    async fn set_battery_trip_point(&self, battery_id: DeviceId, btp: Btp) -> Result<(), BatteryError> {
        self.inner.context.btp_handler(battery_id, btp).await
    }

    async fn is_in_use(&self, battery_id: DeviceId) -> Result<PsrReturn, BatteryError> {
        self.inner.context.psr_handler(battery_id).await
    }

    async fn power_source_information(&self, power_source_id: DeviceId) -> Result<PifFixedStrings, BatteryError> {
        self.inner.context.pif_handler(power_source_id).await
    }

    async fn device_status(&self, battery_id: DeviceId) -> Result<StaReturn, BatteryError> {
        self.inner.context.sta_handler(battery_id).await
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
