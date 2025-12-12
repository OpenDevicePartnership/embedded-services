use crate::AcpiBatteryError;
use crate::device::{self};
use crate::device::{Device, FuelGaugeError};
use battery_service_messages::{AcpiBatteryRequest, AcpiBatteryResponse, DeviceId};
use embassy_sync::channel::Channel;
use embassy_sync::channel::TrySendError;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::comms::MailboxDelegateError;
use embedded_services::power::policy::PowerCapability;
use embedded_services::{IntrusiveList, debug, error, info, intrusive_list, trace, warn};

use core::ops::DerefMut;
use core::sync::atomic::AtomicUsize;

/// Battery service states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    NotPresent,

    Present(PresentSubstate),
}

/// Present state substates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PresentSubstate {
    NotOperational,
    Operational(OperationalSubstate),
}

/// Operational state substates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OperationalSubstate {
    Init,
    Polling,
}

/// Battery state machine events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BatteryEventInner {
    /// Send this command to initialize or re-initialize the state machine.
    DoInit,
    /// Send this command while in the Present(Operational(Polling)) state to request the fuel gauge to poll dynamic data.
    PollDynamicData,
    /// Send this command while in the Present(Operational(Init)) or Present(Operational(Polling)) state to request the fuel gauge to poll static data.
    PollStaticData,
    /// Send this command while in any state to put the state machine into the Present(NotOperational) state.
    /// The state machine will ping the FG and if the ping succeeds, the state machine will drop into the
    /// Present(Operational(Init)) state, where you can send PollStaticData to get it into a polling state.
    /// If there is a failure, this command can be sent multiple times. Once enough failures have occured, the state
    /// machine will send a NoOpRecoveryFailed error and will drop into the NotPresent state. At that point, the state
    /// machine must be reinitialized with a DoInit command.
    Timeout,
    Oem(u8, &'static [u8]),
}

/// Battery state machine response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InnerStateMachineResponse {
    Complete,
}

/// Battery state machine errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StateMachineError {
    DeviceTimeout,
    DeviceError,
    InvalidActionInState,
    NoOpRecoveryFailed,
}

/// External battery state machine response.  
type StateMachineResponse = Result<InnerStateMachineResponse, StateMachineError>;

/// Battery service context response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContextResponse {
    Ack,
}

/// Battery service context error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContextError {
    DeviceNotFound,
    Timeout,
    StateError(StateMachineError),
    DriverError(FuelGaugeError),
}

/// External battery service context response.
pub type BatteryResponse = Result<ContextResponse, ContextError>;

/// External battery state machine event wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BatteryEvent {
    pub event: BatteryEventInner,
    pub device_id: DeviceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct PsuState {
    pub psu_connected: bool,
    pub power_capability: Option<PowerCapability>,
}

impl PsuState {
    pub const fn new() -> Self {
        Self {
            psu_connected: false,
            power_capability: None,
        }
    }
}

impl Default for PsuState {
    fn default() -> Self {
        Self::new()
    }
}

/// Battery service context, hardware agnostic state.
pub struct Context {
    fuel_gauges: IntrusiveList,
    state: Mutex<GlobalRawMutex, State>,
    battery_event: Channel<GlobalRawMutex, BatteryEvent, 1>,
    battery_response: Channel<GlobalRawMutex, BatteryResponse, 1>,
    no_op_retry_count: AtomicUsize,
    config: Config,
    acpi_request: Signal<GlobalRawMutex, AcpiBatteryRequest>,
    power_info: Mutex<GlobalRawMutex, PsuState>,
}

pub struct Config {
    state_machine_timeout_ms: Duration,
    no_op_max_retries: usize,
}

impl Config {
    pub const fn new() -> Self {
        Self {
            state_machine_timeout_ms: Duration::from_secs(120),
            no_op_max_retries: 5,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Create a new context instance.
    pub fn new() -> Self {
        Self::new_inner(Default::default())
    }

    pub const fn new_with_config(config: Config) -> Self {
        Self::new_inner(config)
    }

    const fn new_inner(config: Config) -> Self {
        Self {
            fuel_gauges: IntrusiveList::new(),
            state: Mutex::new(State::NotPresent),
            battery_event: Channel::new(),
            battery_response: Channel::new(),
            no_op_retry_count: AtomicUsize::new(0),
            config,
            acpi_request: Signal::new(),
            power_info: Mutex::new(PsuState::new()),
        }
    }

    /// Get global state machine timeout.
    fn get_state_machine_timeout(&self) -> Duration {
        self.config.state_machine_timeout_ms
    }

    /// Get global state machine NotOperational max # of retries.
    fn get_state_machine_max_retries(&self) -> usize {
        self.config.no_op_max_retries
    }

    /// Get global state machine NotOperational retry count.
    fn get_state_machine_retry_count(&self) -> usize {
        self.no_op_retry_count.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Set global state machine NotOperational retry count.
    fn set_state_machine_retry_count(&self, retry_count: usize) {
        self.no_op_retry_count
            .store(retry_count, core::sync::atomic::Ordering::Relaxed)
    }

    /// Main processing function.
    pub async fn process(&self, event: BatteryEvent) {
        let res = with_timeout(self.get_state_machine_timeout(), self.do_state_machine(event)).await;
        match res {
            Ok(sm_res) => match sm_res {
                Ok(_) => {
                    debug!("Battery state machine completed for event {:?}", event);
                    self.battery_response.send(Ok(ContextResponse::Ack)).await;
                }
                Err(e) => {
                    error!("Battery state machine completed but errored {:?}", event);
                    self.battery_response.send(Err(ContextError::StateError(e))).await;
                }
            },
            Err(_) => {
                error!("Battery state machine timeout!");

                match self
                    .do_state_machine(BatteryEvent {
                        event: BatteryEventInner::Timeout,
                        device_id: event.device_id,
                    })
                    .await
                {
                    Ok(_) => {
                        self.battery_response.send(Err(ContextError::Timeout)).await;
                    }
                    Err(e) => {
                        self.battery_response.send(Err(ContextError::StateError(e))).await;
                    }
                }
            }
        };
    }

    /// Process and validate event before running state machine.
    fn handle_event(&self, state: &mut State, event: BatteryEventInner) -> Result<State, StateMachineError> {
        match event {
            BatteryEventInner::DoInit => {
                if *state != State::NotPresent {
                    warn!(
                        "Battery Service: received init command when not in init state. State machine reinitializing"
                    );
                    trace!("State = {:?}", *state);
                }
                Ok(State::NotPresent)
            }
            BatteryEventInner::PollDynamicData => {
                if *state != State::Present(PresentSubstate::Operational(OperationalSubstate::Polling)) {
                    error!("Battery Service: received dynamic poll request while not in polling state");
                    trace!("State = {:?}", *state);
                    Err(StateMachineError::InvalidActionInState)
                } else {
                    Ok(State::Present(PresentSubstate::Operational(
                        OperationalSubstate::Polling,
                    )))
                }
            }
            BatteryEventInner::PollStaticData => {
                if *state != State::Present(PresentSubstate::Operational(OperationalSubstate::Init))
                    && *state != State::Present(PresentSubstate::Operational(OperationalSubstate::Polling))
                {
                    error!("Battery Service: received static poll request while not in operational state");
                    trace!("State = {:?}", *state);
                    Err(StateMachineError::InvalidActionInState)
                } else {
                    Ok(State::Present(PresentSubstate::Operational(OperationalSubstate::Init)))
                }
            }
            BatteryEventInner::Timeout => {
                warn!("Battery Service: received timeout command");
                if *state == State::NotPresent {
                    error!(
                        "Battery Service: received timeout command when battery is not present! Re-initialize the battery instead."
                    );
                    Err(StateMachineError::InvalidActionInState)
                } else {
                    Ok(State::Present(PresentSubstate::NotOperational))
                }
            }
            BatteryEventInner::Oem(_, _items) => Err(StateMachineError::InvalidActionInState),
        }
    }

    /// Main battery service state machine
    async fn do_state_machine(&self, event: BatteryEvent) -> StateMachineResponse {
        let mut state = self.state.lock().await;

        // BatteryEventInner can transition state, or an invalid event can cause the state machine to return
        match self.handle_event(state.deref_mut(), event.event) {
            Ok(new_state) => *state = new_state,
            Err(err) => return Err(err),
        }

        match *state {
            State::NotPresent => {
                info!("Initializing fuel gauge with ID {:?}", event.device_id);
                if let Err(e) = self
                    .execute_device_command(event.device_id, device::Command::Ping)
                    .await
                {
                    error!("Error pinging fuel gauge with ID {:?}, {:?}", event.device_id, e);
                    return Err(StateMachineError::DeviceError);
                }
                if let Err(e) = self
                    .execute_device_command(event.device_id, device::Command::Initialize)
                    .await
                {
                    error!("Error initializing fuel gauge with ID {:?}, {:?}", event.device_id, e);
                    return Err(StateMachineError::DeviceError);
                }

                *state = State::Present(PresentSubstate::Operational(OperationalSubstate::Init));
                Ok(InnerStateMachineResponse::Complete)
            }
            State::Present(substate) => match substate {
                PresentSubstate::NotOperational => {
                    self.set_state_machine_retry_count(self.get_state_machine_max_retries() + 1);
                    match self
                        .execute_device_command(event.device_id, device::Command::Ping)
                        .await
                    {
                        Ok(device::InternalResponse::Complete) => {
                            info!("Fuel gauge id: {:?} re-established communication!", event.device_id);
                            *state = State::Present(PresentSubstate::Operational(OperationalSubstate::Init));
                            self.set_state_machine_retry_count(0);
                            Ok(InnerStateMachineResponse::Complete)
                            // Do not continue execution.
                        }
                        Err(ContextError::DriverError(fg_err)) => {
                            error!(
                                "Fuel gauge {:?} failed to ping with error {:?}",
                                event.device_id, fg_err
                            );
                            // Do not continue execution, if we got to this point it's because we errored.
                            // Require re-executing manual Timeout calls. If we go over the max retries,
                            // transition to the NotPresent state.
                            if self.get_state_machine_retry_count() > self.get_state_machine_max_retries() {
                                *state = State::NotPresent;
                                return Err(StateMachineError::NoOpRecoveryFailed);
                            }
                            Err(StateMachineError::DeviceTimeout)
                        }
                        Err(ctx_err) => {
                            error!(
                                "Battery state machine NotOperational error: {:?} for ID {:?}",
                                ctx_err, event.device_id
                            );
                            // Do not continue execution, if we got to this point it's because we errored.
                            // Require re-executing manual Timeout calls. If we go over the max retries,
                            // transition to the NotPresent state.
                            if self.get_state_machine_retry_count() > self.get_state_machine_max_retries() {
                                *state = State::NotPresent;
                                return Err(StateMachineError::NoOpRecoveryFailed);
                            }
                            Err(StateMachineError::DeviceTimeout)
                        }
                    }
                }
                PresentSubstate::Operational(operational_substate) => match operational_substate {
                    OperationalSubstate::Init => {
                        // Collect static data
                        trace!("Collecting fuel gauge static cache with ID {:?}", event.device_id);
                        if let Err(e) = self
                            .execute_device_command(event.device_id, device::Command::UpdateStaticCache)
                            .await
                        {
                            error!(
                                "Error updating fuel gauge static cache with ID {:?}, {:?}",
                                event.device_id, e
                            );
                            return Err(StateMachineError::DeviceError);
                        }
                        *state = State::Present(PresentSubstate::Operational(OperationalSubstate::Polling));
                        Ok(InnerStateMachineResponse::Complete)
                    }
                    OperationalSubstate::Polling => {
                        // Collect dynamic data
                        trace!("Collecting fuel gauge dynamic cache with ID {:?}", event.device_id);
                        if let Err(e) = self
                            .execute_device_command(event.device_id, device::Command::UpdateDynamicCache)
                            .await
                        {
                            error!(
                                "Error initializing fuel gauge dynamic cache with ID {:?}, {:?}",
                                event.device_id, e
                            );
                            return Err(StateMachineError::DeviceError);
                        }
                        Ok(InnerStateMachineResponse::Complete)
                    }
                },
            },
        }
    }

    pub(super) async fn process_acpi_cmd(&self, acpi_msg: &AcpiBatteryRequest) {
        let response: Result<AcpiBatteryResponse, AcpiBatteryError> = match *acpi_msg {
            AcpiBatteryRequest::BatteryGetBixRequest { battery_id } => self.bix_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatteryGetBstRequest { battery_id } => self.bst_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatteryGetPsrRequest { battery_id } => self.psr_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatteryGetPifRequest { battery_id } => self.pif_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatteryGetBpsRequest { battery_id } => self.bps_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatterySetBtpRequest { battery_id, btp } => {
                self.btp_handler(DeviceId(battery_id), btp).await
            }
            AcpiBatteryRequest::BatterySetBptRequest { battery_id, bpt } => {
                self.bpt_handler(DeviceId(battery_id), bpt).await
            }
            AcpiBatteryRequest::BatteryGetBpcRequest { battery_id } => self.bpc_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatterySetBmcRequest { battery_id, bmc } => {
                self.bmc_handler(DeviceId(battery_id), bmc).await
            }
            AcpiBatteryRequest::BatteryGetBmdRequest { battery_id } => self.bmd_handler(DeviceId(battery_id)).await,
            AcpiBatteryRequest::BatteryGetBctRequest { battery_id, bct } => {
                self.bct_handler(DeviceId(battery_id), bct).await
            }
            AcpiBatteryRequest::BatteryGetBtmRequest { battery_id, btm } => {
                self.btm_handler(DeviceId(battery_id), btm).await
            }

            AcpiBatteryRequest::BatterySetBmsRequest { battery_id, bms } => {
                self.bms_handler(DeviceId(battery_id), bms).await
            }
            AcpiBatteryRequest::BatterySetBmaRequest { battery_id, bma } => {
                self.bma_handler(DeviceId(battery_id), bma).await
            }
            AcpiBatteryRequest::BatteryGetStaRequest { battery_id } => self.sta_handler(DeviceId(battery_id)).await,
        };

        if let Err(e) = response {
            error!("Battery service command failed: {:?}", e);
        }

        // TODO We should probably be responding to the requestor rather than just assuming the request came from the host
        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &response,
        )
        .await
        .expect("comms_send is infallible");
    }

    pub(crate) fn get_fuel_gauge(&self, id: DeviceId) -> Option<&'static Device> {
        for device in &self.fuel_gauges {
            if let Some(data) = device.data::<Device>() {
                if data.id() == id {
                    return Some(data);
                }
            } else {
                error!("Non-device located in devices list");
            }
        }
        None
    }

    /// Register fuel gauge device with the context instance.
    pub fn register_fuel_gauge(&self, device: &'static Device) -> Result<(), intrusive_list::Error> {
        if self.get_fuel_gauge(device.id()).is_some() {
            return Err(embedded_services::Error::NodeAlreadyInList);
        }

        self.fuel_gauges.push(device)
    }

    async fn send_event(&self, event: BatteryEvent) {
        self.battery_event.send(event).await;
    }

    pub async fn wait_response(&self) -> BatteryResponse {
        self.battery_response.receive().await
    }

    /// Send an event to the context and wait for a response.
    pub async fn execute_event(&self, event: BatteryEvent) -> BatteryResponse {
        self.send_event(event).await;
        self.wait_response().await
    }

    pub fn send_event_no_wait(&self, event: BatteryEvent) -> Result<(), TrySendError<BatteryEvent>> {
        self.battery_event.try_send(event)
    }

    /// Wait for battery event.
    pub async fn wait_event(&self) -> BatteryEvent {
        self.battery_event.receive().await
    }

    pub(super) fn send_acpi_cmd(&self, request: AcpiBatteryRequest) {
        self.acpi_request.signal(request);
    }

    pub(super) async fn wait_acpi_cmd(&self) -> AcpiBatteryRequest {
        self.acpi_request.wait().await
    }

    pub async fn get_state(&self) -> State {
        *self.state.lock().await
    }

    async fn execute_device_command(
        &self,
        id: DeviceId,
        command: device::Command,
    ) -> Result<device::InternalResponse, ContextError> {
        // Get ID
        let device = match self.get_fuel_gauge(id) {
            Some(device) => device,
            None => {
                // TODO: Send error response
                error!("Fuel gauge with ID {:?} not found", id);
                return Err(ContextError::DeviceNotFound);
            }
        };

        match with_timeout(device.get_timeout(), device.execute_command(command)).await {
            Ok(res) => match res {
                Ok(response) => Ok(response),
                Err(e) => Err(ContextError::DriverError(e)),
            },
            Err(_) => {
                error!("Device timed out when executing command {:?}", command);
                Err(ContextError::Timeout)
            }
        }
    }

    pub(crate) async fn get_power_info(&self) -> PsuState {
        *self.power_info.lock().await
    }

    pub(crate) fn set_power_info(
        &self,
        power_info: &embedded_services::power::policy::CommsData,
    ) -> Result<(), MailboxDelegateError> {
        let mut guard = self
            .power_info
            .try_lock()
            .map_err(|_| MailboxDelegateError::BufferFull)?;

        let psu_state = guard.deref_mut();

        match power_info {
            embedded_services::power::policy::CommsData::ConsumerDisconnected(_) => {
                *psu_state = PsuState {
                    psu_connected: false,
                    power_capability: None,
                }
            }
            embedded_services::power::policy::CommsData::ConsumerConnected(_device_id, power_capability) => {
                *psu_state = PsuState {
                    psu_connected: true,
                    power_capability: Some(power_capability.capability),
                }
            }
            _rest => { /* Don't care about anything else */ }
        }

        trace!("Battery: PSU state: {:?}", psu_state);
        Ok(())
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}
