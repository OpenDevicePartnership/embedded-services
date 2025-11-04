//! Device struct and methods
use embassy_sync::mutex::Mutex;

use super::{DeviceId, Error};
use crate::event::Receiver;
use crate::power::policy::policy::RequestData;
use crate::power::policy::{ConsumerPowerCapability, ProviderPowerCapability};
use crate::sync::Lockable;
use crate::{GlobalRawMutex, intrusive_list};

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

/// Internal device state for power policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    /// Current state of the device
    pub state: State,
    /// Current consumer capability
    pub consumer_capability: Option<ConsumerPowerCapability>,
    /// Current requested provider capability
    pub requested_provider_capability: Option<ProviderPowerCapability>,
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

/// Trait for devices that can be controlled by a power policy implementation
pub trait DeviceTrait {
    /// Disconnect power from this device
    fn disconnect(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to provide power to an external connection
    fn connect_provider(&mut self, capability: ProviderPowerCapability) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to consume power from an external connection
    fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> impl Future<Output = Result<(), Error>>;
    /// Device is out of sync with the policy, reset and renotify power policy
    fn reset(&mut self) -> impl Future<Output = Result<(), Error>>;
}

/// Device struct
pub struct Device<'a, C: Lockable, R: Receiver<RequestData>>
where
    C::Inner: DeviceTrait,
{
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: DeviceId,
    /// Current state of the device
    pub state: Mutex<GlobalRawMutex, InternalState>,
    /// Reference to hardware
    pub device: &'a C,
    /// Event receiver
    pub receiver: Mutex<GlobalRawMutex, R>,
}

impl<'a, C: Lockable, R: Receiver<RequestData>> Device<'a, C, R>
where
    C::Inner: DeviceTrait,
{
    /// Create a new device
    pub fn new(id: DeviceId, device: &'a C, receiver: R) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Detached,
                consumer_capability: None,
                requested_provider_capability: None,
            }),
            context_ref,
            device,
            receiver: Mutex::new(receiver),
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
    pub async fn consumer_capability(&self) -> Option<ConsumerPowerCapability> {
        self.state.lock().await.consumer_capability
    }

    /// Returns true if the device is currently consuming power
    pub async fn is_consumer(&self) -> bool {
        self.state().await.kind() == StateKind::ConnectedConsumer
    }

    /// Returns current provider power capability
    pub async fn provider_capability(&self) -> Option<ProviderPowerCapability> {
        match self.state().await {
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
        self.state().await.kind() == StateKind::ConnectedProvider
    }
}

impl<C: Lockable, R: Receiver<RequestData> + 'static> intrusive_list::NodeContainer for Device<'static, C, R>
where
    C::Inner: DeviceTrait,
{
    fn get_node(&self) -> &crate::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait DeviceContainer<C: Lockable, R: Receiver<RequestData>>
where
    C::Inner: DeviceTrait,
{
    /// Get the underlying device struct
    fn get_power_policy_device(&self) -> &Device<'_, C, R>;
}

impl<C: Lockable, R: Receiver<RequestData>> DeviceContainer<C, R> for Device<'_, C, R>
where
    C::Inner: DeviceTrait,
{
    fn get_power_policy_device(&self) -> &Device<'_, C, R> {
        self
    }
}
