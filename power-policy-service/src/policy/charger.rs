//! Charger device struct and controller
use core::{future::Future, ops::DerefMut};

use embassy_sync::{channel::Channel, mutex::Mutex};
use embedded_services::{GlobalRawMutex, intrusive_list};

use super::ConsumerPowerCapability;
use super::PowerCapability;

/// Charger controller trait that device drivers may use to integrate with internal messaging system
pub trait ChargeController: embedded_batteries_async::charger::Charger {
    /// Type of error returned by the bus
    type ChargeControllerError;

    /// Returns with pending events
    fn wait_event(&mut self) -> impl Future<Output = ChargerEvent>;
    /// Initialize charger hardware, after this returns the charger should be ready to charge
    fn init_charger(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Returns if the charger hardware detects if a PSU is attached
    fn is_psu_attached(&mut self) -> impl Future<Output = Result<bool, Self::ChargeControllerError>>;
    /// Called after power policy attaches to a power port.
    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Called after power policy detaches from a power port, either to switch consumers,
    /// or because PSU was disconnected.
    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Called when a charger CheckReady request (PolicyEvent::CheckReady) is sent to the power policy.
    /// Upon successful return of this method, the charger is assumed to be powered and ready to communicate,
    /// transitioning state from unpowered to powered.
    ///
    /// If the charger is powered, an Ok(()) does nothing. An Err(_) will put the charger into an
    /// unpowered state, meaning another PolicyEvent::CheckReady must be sent to re-establish communications
    /// with the charger. Upon successful return, the charger must be re-initialized by sending a
    /// `PolicyEvent::InitRequest`.
    fn is_ready(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>> {
        core::future::ready(Ok(()))
    }
}

/// Charger Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChargerId(pub u8);

/// PSU state as determined by charger device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PsuState {
    /// Charger detected PSU attached
    Attached,
    /// Charger detected PSU detached
    Detached,
}

impl From<bool> for PsuState {
    fn from(value: bool) -> Self {
        match value {
            true => PsuState::Attached,
            false => PsuState::Detached,
        }
    }
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerEvent {
    /// Charger finished initialization sequence
    Initialized(PsuState),
    /// PSU state changed
    PsuStateChange(PsuState),
    /// A timeout of some sort was detected
    Timeout,
    /// An error occured on the bus
    BusError,
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

impl From<ChargerError> for crate::policy::Error {
    fn from(value: ChargerError) -> Self {
        Self::Charger(value)
    }
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PolicyEvent {
    /// Request to initialize charger hardware
    InitRequest,
    /// New power policy detected
    PolicyConfiguration(ConsumerPowerCapability),
    /// Request to check if the charger hardware is ready to receive communications.
    /// For example, if the charger is powered.
    CheckReady,
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerResponseData {
    /// Command completed
    Ack,
    /// Charger Unpowered, but we are still Ok
    UnpoweredAck,
}

/// Response for charger requests from policy commands
pub type ChargerResponse = Result<ChargerResponseData, ChargerError>;

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is unpowered
    Unpowered,
    /// Device is powered
    Powered(PoweredSubstate),
}

/// Powered state substates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PoweredSubstate {
    /// Device is initializing
    Init,
    /// PSU is attached and device can charge if desired
    PsuAttached,
    /// PSU is detached
    PsuDetached,
}

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    /// Charger device state
    pub state: State,
    /// Current charger capability
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
    state: Mutex<GlobalRawMutex, InternalState>,
    /// Channel for requests to the device
    commands: Channel<GlobalRawMutex, PolicyEvent, CHARGER_CHANNEL_SIZE>,
    /// Channel for responses from the device
    response: Channel<GlobalRawMutex, ChargerResponse, CHARGER_CHANNEL_SIZE>,
}

impl Device {
    /// Create a new device
    pub fn new(id: ChargerId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Unpowered,
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
    fn get_node(&self) -> &intrusive_list::Node {
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
