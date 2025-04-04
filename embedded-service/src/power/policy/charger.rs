use core::{future::Future, ops::DerefMut};

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel, mutex::Mutex};

use crate::intrusive_list;

use super::PowerCapability;

/// Charger controller trait that device drivers may use to integrate with internal messaging system
pub trait ChargeController: embedded_batteries_async::charger::Charger {
    /// Type of error returned by the bus
    type BusError;

    /// Returns with pending events
    fn wait_event(&mut self) -> impl Future<Output = ChargerEvent>;
    /// Initialize charger hardware, after this returns the charger should be ready to charge
    fn init_charger(&mut self) -> impl Future<Output = Result<(), Self::BusError>>;
    /// Returns if the charger hardware detects if a PSU is attached
    fn is_psu_attached(&mut self) -> impl Future<Output = Result<bool, Self::BusError>>;
}

/// Charger Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChargerId(pub u8);

/// OEM-specific state IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OemStateId(pub u8);

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerEvent {
    /// Charger finished initialization sequence
    Initialized,
    /// PSU attached and we want to switch to it
    PsuAttached,
    /// PSU detached
    PsuDetached,
    /// A timeout of some sort was detected
    Timeout,
    /// An error occured on the bus
    BusError,
    /// OEM specific events
    Oem(OemStateId),
}

/// Charger state errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerError {
    /// Charger received command in an invalid state
    InvalidState(State),
    /// Charger hardware timed out responding
    Timeout,
    /// Charger underlying bus error
    BusError,
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PolicyEvent {
    /// Request to initialize charger hardware
    InitRequest,
    /// PSU attached and we want to switch to it
    PolicyConfiguration(PowerCapability),
    /// OEM specific events
    Oem(OemStateId),
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerResponse {
    /// Command completed
    Ack,
    /// Something went wrong
    Nack(ChargerError),
}

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is initializing
    Init,
    /// Device is waiting for an event
    Idle,
    /// PSU is attached and device can charge if desired
    PsuAttached,
    /// PSU is detached
    PsuDetached,
    // TODO: Dead battery revival?
    /// OEM specific state(s)
    Oem(OemStateId),
}

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    pub state: State,
    pub capability: Option<PowerCapability>,
}

/// Channel size for device requests
pub const CHARGER_CHANNEL_SIZE: usize = 1;

/// Device struct
pub struct Device {
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: ChargerId,
    /// Current state of the device
    state: Mutex<NoopRawMutex, InternalState>,
    /// Channel for requests to the device
    commands: Channel<NoopRawMutex, PolicyEvent, CHARGER_CHANNEL_SIZE>,
    // /// Channel for responses from the device
    response: Channel<NoopRawMutex, ChargerResponse, CHARGER_CHANNEL_SIZE>,
}

impl Device {
    /// Create a new device
    pub fn new(id: ChargerId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Init,
                capability: None,
            }),
            commands: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> ChargerId {
        self.id
    }

    /// Returns the current state of the device
    pub async fn state(&self) -> InternalState {
        *self.state.lock().await
    }

    /// Set the state of the device
    pub async fn set_state(&self, new_state: InternalState) {
        let mut lock = self.state.lock().await;
        let current_state = lock.deref_mut();
        *current_state = new_state;
    }

    /// Wait for a command from policy
    pub async fn wait_command(&self) -> PolicyEvent {
        self.commands.receive().await
    }

    /// Send a command to the charger
    pub async fn send_command(&self, policy_event: PolicyEvent) {
        self.commands.send(policy_event).await
    }

    /// Send a response to the power policy
    pub async fn send_response(&self, response: ChargerResponse) {
        self.response.send(response).await
    }

    /// Send a command and wait for a response from the charger
    pub async fn execute_command(&self, policy_event: PolicyEvent) -> ChargerResponse {
        self.send_command(policy_event).await;
        self.response.receive().await
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &crate::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait ChargerContainer {
    /// Get the underlying device struct
    fn get_charger(&self) -> &Device;
}

impl ChargerContainer for Device {
    fn get_charger(&self) -> &Device {
        self
    }
}
