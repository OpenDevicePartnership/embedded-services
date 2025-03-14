//! Device struct and methods
use core::ops::DerefMut;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;

use super::{action, DeviceId, Error, PowerCapability};
use crate::intrusive_list;

/// Most basic device states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StateKind {
    /// No device attached
    Detached,
    /// Device is attached
    Idle,
    /// Device is actively providing power
    ConnectedProvider,
    /// Device is actively consuming power
    ConnectedConsumer,
}

/// Current state of the power device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is attached, but is not currently providing or consuming power
    Idle,
    /// Device is attached and is currently providing power
    ConnectedProvider(PowerCapability),
    /// Device is attached and is currently consuming power
    ConnectedConsumer(PowerCapability),
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

/// Internal device state for power policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct InternalState {
    /// Current state of the device
    pub state: State,
    /// Current consumer capability
    pub consumer_capability: Option<PowerCapability>,
    /// Device encountered an error and is in the process of recovering
    pub in_recovery: bool,
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Start consuming on this device
    ConnectConsumer(PowerCapability),
    /// Start providinig on this device
    ConnectProvider(PowerCapability),
    /// Stop providing or consuming on this device
    Disconnect,
}

/// Request from power policy service to a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Request {
    /// Target device
    pub id: DeviceId,
    /// Request data
    pub data: RequestData,
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

/// Channel size for device requests
pub const DEVICE_CHANNEL_SIZE: usize = 1;

/// Device struct
pub struct Device {
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: DeviceId,
    /// Current state of the device
    state: Mutex<NoopRawMutex, InternalState>,
    /// Channel for requests to the device
    request: Channel<NoopRawMutex, RequestData, DEVICE_CHANNEL_SIZE>,
    /// Channel for responses from the device
    response: Channel<NoopRawMutex, InternalResponseData, DEVICE_CHANNEL_SIZE>,
}

impl Device {
    /// Create a new device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Detached,
                consumer_capability: None,
                in_recovery: false,
            }),
            request: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Returns the current state of the device
    pub async fn state(&self) -> State {
        self.state.lock().await.state
    }

    /// Returns the current consumer capability of the device
    pub async fn consumer_capability(&self) -> Option<PowerCapability> {
        self.state.lock().await.consumer_capability
    }

    /// Returns true if the device is currently consuming power
    pub async fn is_consumer(&self) -> bool {
        self.state().await.kind() == StateKind::ConnectedConsumer
    }

    /// Returns current provider power capability
    pub async fn provider_capability(&self) -> Option<PowerCapability> {
        match self.state().await {
            State::ConnectedProvider(capability) => Some(capability),
            _ => None,
        }
    }

    /// Returns true if the device is currently providing power
    pub async fn is_provider(&self) -> bool {
        self.state().await.kind() == StateKind::ConnectedProvider
    }

    /// Returns true if the device is currently in recovery
    pub async fn is_in_recovery(&self) -> bool {
        self.state.lock().await.in_recovery
    }

    /// Enter recovery mode
    pub(super) async fn enter_recovery(&self) {
        self.state.lock().await.in_recovery = true;
    }

    /// Exit recovery mode
    pub(super) async fn exit_recovery(&self) {
        self.state.lock().await.in_recovery = false;
    }

    /// Sends a request to this device and returns a response
    pub(super) async fn execute_device_request(&self, request: RequestData) -> Result<ResponseData, Error> {
        self.request.send(request).await;
        self.response.receive().await
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> RequestData {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: InternalResponseData) {
        self.response.send(response).await;
    }

    /// Internal function to set device state
    pub(super) async fn set_state(&self, new_state: State) {
        let mut lock = self.state.lock().await;
        let state = lock.deref_mut();
        state.state = new_state;
    }

    /// Internal function to set consumer capability
    pub(super) async fn update_consumer_capability(&self, capability: Option<PowerCapability>) {
        let mut lock = self.state.lock().await;
        let state = lock.deref_mut();
        state.consumer_capability = capability;
    }

    /// Try to provide access to the device actions for the given state
    pub async fn try_device_action<'a, S: action::Kind>(&'a self) -> Result<action::device::Device<'a, S>, Error> {
        let state = self.state().await.kind();
        if S::kind() != state {
            return Err(Error::InvalidState(S::kind(), state));
        }
        Ok(action::device::Device::new(self))
    }

    /// Provide access to the current device state
    pub async fn device_action<'a>(&'a self) -> action::device::AnyState<'a> {
        match self.state().await.kind() {
            StateKind::Detached => action::device::AnyState::Detached(action::device::Device::new(self)),
            StateKind::Idle => action::device::AnyState::Idle(action::device::Device::new(self)),
            StateKind::ConnectedProvider => {
                action::device::AnyState::ConnectedProvider(action::device::Device::new(self))
            }
            StateKind::ConnectedConsumer => {
                action::device::AnyState::ConnectedConsumer(action::device::Device::new(self))
            }
        }
    }

    /// Try to provide access to the policy actions for the given state
    /// Implemented here for lifetime reasons
    pub(super) async fn try_policy_action<'a, S: action::Kind>(
        &'a self,
    ) -> Result<action::policy::Policy<'a, S>, Error> {
        let state = self.state().await.kind();
        if S::kind() != state {
            return Err(Error::InvalidState(S::kind(), state));
        }
        Ok(action::policy::Policy::new(self))
    }

    /// Provide access to the current policy actions
    /// Implemented here for lifetime reasons
    pub(super) async fn policy_action<'a>(&'a self) -> action::policy::AnyState<'a> {
        match self.state().await.kind() {
            StateKind::Detached => action::policy::AnyState::Detached(action::policy::Policy::new(self)),
            StateKind::Idle => action::policy::AnyState::Idle(action::policy::Policy::new(self)),
            StateKind::ConnectedProvider => {
                action::policy::AnyState::ConnectedProvider(action::policy::Policy::new(self))
            }
            StateKind::ConnectedConsumer => {
                action::policy::AnyState::ConnectedConsumer(action::policy::Policy::new(self))
            }
        }
    }

    /// Detach the device, this action is available in all states
    pub async fn detach<'a>(&'a self) -> Result<action::device::Device<'a, action::Detached>, Error> {
        match self.device_action().await {
            action::device::AnyState::Detached(state) => Ok(state),
            action::device::AnyState::Idle(state) => state.detach().await,
            action::device::AnyState::ConnectedProvider(state) => state.detach().await,
            action::device::AnyState::ConnectedConsumer(state) => state.detach().await,
        }
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &crate::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait DeviceContainer {
    /// Get the underlying device struct
    fn get_power_policy_device(&self) -> &Device;
}

impl DeviceContainer for Device {
    fn get_power_policy_device(&self) -> &Device {
        self
    }
}
