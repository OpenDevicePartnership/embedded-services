//! Device struct and methods
use embassy_sync::mutex::Mutex;

use super::{DeviceId, Error};
use crate::policy::policy::RequestData;
use crate::policy::{ConsumerPowerCapability, ProviderPowerCapability};
use embedded_services::event::Receiver;
use embedded_services::sync::Lockable;
use embedded_services::{GlobalRawMutex, intrusive_list};

/// Most basic device states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StateKind {
    /// No device attached
    Detached,
    /// Device is attached
    Idle,
    /// Device is actively providing power, USB PD source mode
    ConnectedProvider,
    /// Device is actively consuming power, USB PD sink mode
    ConnectedConsumer,
}

/// Current state of the power device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is attached, but is not currently providing or consuming power
    Idle,
    /// Device is attached and is currently providing power
    ConnectedProvider(ProviderPowerCapability),
    /// Device is attached and is currently consuming power
    ConnectedConsumer(ConsumerPowerCapability),
    /// No device attached
    Detached,
}

impl State {
    /// Returns the correpsonding state kind
    pub fn kind(&self) -> StateKind {
        match self {
            State::Idle => StateKind::Idle,
            State::ConnectedProvider(_) => StateKind::ConnectedProvider,
            State::ConnectedConsumer(_) => StateKind::ConnectedConsumer,
            State::Detached => StateKind::Detached,
        }
    }
}

/// Per-device state for power policy implementation
///
/// This struct implements the state machine outlined in the docs directory.
/// The various state transition functions always succeed in the sense that
/// the desired state is always entered, but some still return a result.
/// This is because a the device that is driving this state machine is the
/// ultimate source of truth and the recovery procedure would ultimately
/// end up catching up to this state anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    /// Current state of the device
    state: State,
    /// Current consumer capability
    consumer_capability: Option<ConsumerPowerCapability>,
    /// Current requested provider capability
    requested_provider_capability: Option<ProviderPowerCapability>,
}

impl Default for InternalState {
    fn default() -> Self {
        Self {
            state: State::Detached,
            consumer_capability: None,
            requested_provider_capability: None,
        }
    }
}

impl InternalState {
    /// Attach the device
    pub fn attach(&mut self) -> Result<(), Error> {
        let result = if self.state == State::Detached {
            Ok(())
        } else {
            Err(Error::InvalidState(&[StateKind::Detached], self.state.kind()))
        };
        self.state = State::Idle;
        result
    }

    /// Detach the device
    ///
    /// Detach is always a valid transition
    pub fn detach(&mut self) {
        self.state = State::Detached;
        self.consumer_capability = None;
        self.requested_provider_capability = None;
    }

    /// Disconnect this device
    pub fn disconnect(&mut self, clear_caps: bool) -> Result<(), Error> {
        let result = if matches!(self.state, State::ConnectedConsumer(_) | State::ConnectedProvider(_)) {
            Ok(())
        } else {
            Err(Error::InvalidState(
                &[StateKind::ConnectedConsumer, StateKind::ConnectedProvider],
                self.state.kind(),
            ))
        };
        self.state = State::Idle;
        if clear_caps {
            self.consumer_capability = None;
            self.requested_provider_capability = None;
        }
        result
    }

    /// Update the available consumer capability
    pub fn update_consumer_power_capability(
        &mut self,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        let result = match self.state {
            State::Idle | State::ConnectedConsumer(_) | State::ConnectedProvider(_) => Ok(()),
            _ => Err(Error::InvalidState(
                &[
                    StateKind::Idle,
                    StateKind::ConnectedConsumer,
                    StateKind::ConnectedProvider,
                ],
                self.state.kind(),
            )),
        };
        self.consumer_capability = capability;
        result
    }

    /// Update the requested provider capability
    pub fn update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), Error> {
        if self.requested_provider_capability == capability {
            // Already operating at this capability, power policy is already aware, don't need to do anything
            return Ok(());
        }

        let result = match self.state {
            State::Idle | State::ConnectedConsumer(_) | State::ConnectedProvider(_) => Ok(()),
            _ => Err(Error::InvalidState(
                &[
                    StateKind::Idle,
                    StateKind::ConnectedProvider,
                    StateKind::ConnectedConsumer,
                ],
                self.state.kind(),
            )),
        };

        self.requested_provider_capability = capability;
        result
    }

    /// Handle a request to connect as a consumer from the policy
    pub fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        let result = if self.state == State::Idle {
            Ok(())
        } else {
            Err(Error::InvalidState(&[StateKind::Idle], self.state.kind()))
        };
        self.state = State::ConnectedConsumer(capability);
        result
    }

    /// Handle a request to connect as a provider from the policy
    pub fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        let result = if matches!(self.state, State::Idle | State::ConnectedProvider(_)) {
            Ok(())
        } else {
            Err(Error::InvalidState(
                &[StateKind::Idle, StateKind::ConnectedProvider],
                self.state.kind(),
            ))
        };
        self.state = State::ConnectedProvider(capability);
        result
    }

    /// Returns the current state machine state
    pub fn state(&self) -> State {
        self.state
    }

    /// Returns the current consumer capability
    pub fn consumer_capability(&self) -> Option<ConsumerPowerCapability> {
        self.consumer_capability
    }

    /// Returns the requested provider capability
    pub fn requested_provider_capability(&self) -> Option<ProviderPowerCapability> {
        self.requested_provider_capability
    }
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommandData {
    /// Start consuming on this device
    ConnectAsConsumer(ConsumerPowerCapability),
    /// Start providing power to port partner on this device
    ConnectAsProvider(ProviderPowerCapability),
    /// Stop providing or consuming on this device
    Disconnect,
}

/// Request from power policy service to a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Command {
    /// Target device
    pub id: DeviceId,
    /// Request data
    pub data: CommandData,
}

/// Data for a device response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// The request was successful
    Complete,
}

impl ResponseData {
    /// Returns an InvalidResponse error if the response is not complete
    pub fn complete_or_err(self) -> Result<(), Error> {
        match self {
            ResponseData::Complete => Ok(()),
        }
    }
}

/// Wrapper type to make code cleaner
pub type InternalResponseData = Result<ResponseData, Error>;

/// Response from a device to the power policy service
pub struct Response {
    /// Target device
    pub id: DeviceId,
    /// Response data
    pub data: ResponseData,
}

/// Trait for PSU devices
pub trait DeviceTrait {
    /// Disconnect power from this device
    fn disconnect(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to provide power to an external connection
    fn connect_provider(&mut self, capability: ProviderPowerCapability) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to consume power from an external connection
    fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> impl Future<Output = Result<(), Error>>;
}

/// PSU registration struct
pub struct Device<'a, D: Lockable, R: Receiver<RequestData>>
where
    D::Inner: DeviceTrait,
{
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: DeviceId,
    /// Current state of the device
    pub state: Mutex<GlobalRawMutex, InternalState>,
    /// Reference to hardware
    pub device: &'a D,
    /// Event receiver
    pub receiver: Mutex<GlobalRawMutex, R>,
}

impl<'a, D: Lockable, R: Receiver<RequestData>> Device<'a, D, R>
where
    D::Inner: DeviceTrait,
{
    /// Create a new device
    pub fn new(id: DeviceId, device: &'a D, receiver: R) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Detached,
                consumer_capability: None,
                requested_provider_capability: None,
            }),
            device,
            receiver: Mutex::new(receiver),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Returns the current consumer capability of the device
    pub async fn consumer_capability(&self) -> Option<ConsumerPowerCapability> {
        self.state.lock().await.consumer_capability
    }

    /// Returns true if the device is currently consuming power
    pub async fn is_consumer(&self) -> bool {
        self.state.lock().await.state.kind() == StateKind::ConnectedConsumer
    }

    /// Returns current provider power capability
    pub async fn provider_capability(&self) -> Option<ProviderPowerCapability> {
        match self.state.lock().await.state {
            State::ConnectedProvider(capability) => Some(capability),
            _ => None,
        }
    }

    /// Returns the current requested provider capability
    pub async fn requested_provider_capability(&self) -> Option<ProviderPowerCapability> {
        self.state.lock().await.requested_provider_capability
    }

    /// Returns true if the device is currently providing power
    pub async fn is_provider(&self) -> bool {
        self.state.lock().await.state.kind() == StateKind::ConnectedProvider
    }
}

impl<D: Lockable, R: Receiver<RequestData> + 'static> intrusive_list::NodeContainer for Device<'static, D, R>
where
    D::Inner: DeviceTrait,
{
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait DeviceContainer<D: Lockable, R: Receiver<RequestData>>
where
    D::Inner: DeviceTrait,
{
    /// Get the underlying device struct
    fn get_power_policy_device(&self) -> &Device<'_, D, R>;
}

impl<D: Lockable, R: Receiver<RequestData>> DeviceContainer<D, R> for Device<'_, D, R>
where
    D::Inner: DeviceTrait,
{
    fn get_power_policy_device(&self) -> &Device<'_, D, R> {
        self
    }
}
