//! Device struct and methods
use crate::capability::{ConsumerPowerCapability, PowerCapability, ProviderPowerCapability};
use embedded_services::sync::Lockable;

pub mod event;

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The requested device does not exist
    InvalidDevice,
    /// The provide request was denied, contains maximum available power
    CannotProvide(Option<PowerCapability>),
    /// The consume request was denied, contains maximum available power
    CannotConsume(Option<PowerCapability>),
    /// The device is not in the correct state (expected, actual)
    InvalidState(&'static [StateKind], StateKind),
    /// Invalid response
    InvalidResponse,
    /// Busy, the device cannot respond to the request at this time
    Busy,
    /// Timeout
    Timeout,
    /// Bus error
    Bus,
    /// Charger specific error, underlying error should have more context
    Charger(crate::charger::ChargerError),
    /// Generic failure
    Failed,
}

/// Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

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
pub enum PsuState {
    /// Device is attached, but is not currently providing or consuming power
    Idle,
    /// Device is attached and is currently providing power
    ConnectedProvider(ProviderPowerCapability),
    /// Device is attached and is currently consuming power
    ConnectedConsumer(ConsumerPowerCapability),
    /// No device attached
    Detached,
}

impl PsuState {
    /// Returns the correpsonding state kind
    pub fn kind(&self) -> StateKind {
        match self {
            PsuState::Idle => StateKind::Idle,
            PsuState::ConnectedProvider(_) => StateKind::ConnectedProvider,
            PsuState::ConnectedConsumer(_) => StateKind::ConnectedConsumer,
            PsuState::Detached => StateKind::Detached,
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
pub struct State {
    /// Current state of the device
    pub psu_state: PsuState,
    /// Current consumer capability
    pub consumer_capability: Option<ConsumerPowerCapability>,
    /// Current requested provider capability
    pub requested_provider_capability: Option<ProviderPowerCapability>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            psu_state: PsuState::Detached,
            consumer_capability: None,
            requested_provider_capability: None,
        }
    }
}

impl State {
    /// Attach the device
    pub fn attach(&mut self) -> Result<(), Error> {
        let result = if self.psu_state == PsuState::Detached {
            Ok(())
        } else {
            Err(Error::InvalidState(&[StateKind::Detached], self.psu_state.kind()))
        };
        self.psu_state = PsuState::Idle;
        result
    }

    /// Detach the device
    ///
    /// Detach is always a valid transition
    pub fn detach(&mut self) {
        self.psu_state = PsuState::Detached;
        self.consumer_capability = None;
        self.requested_provider_capability = None;
    }

    /// Disconnect this device
    pub fn disconnect(&mut self, clear_caps: bool) -> Result<(), Error> {
        let result = if matches!(
            self.psu_state,
            PsuState::ConnectedConsumer(_) | PsuState::ConnectedProvider(_)
        ) {
            Ok(())
        } else {
            Err(Error::InvalidState(
                &[StateKind::ConnectedConsumer, StateKind::ConnectedProvider],
                self.psu_state.kind(),
            ))
        };
        self.psu_state = PsuState::Idle;
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
        let result = match self.psu_state {
            PsuState::Idle | PsuState::ConnectedConsumer(_) | PsuState::ConnectedProvider(_) => Ok(()),
            _ => Err(Error::InvalidState(
                &[
                    StateKind::Idle,
                    StateKind::ConnectedConsumer,
                    StateKind::ConnectedProvider,
                ],
                self.psu_state.kind(),
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

        let result = match self.psu_state {
            PsuState::Idle | PsuState::ConnectedConsumer(_) | PsuState::ConnectedProvider(_) => Ok(()),
            _ => Err(Error::InvalidState(
                &[
                    StateKind::Idle,
                    StateKind::ConnectedProvider,
                    StateKind::ConnectedConsumer,
                ],
                self.psu_state.kind(),
            )),
        };

        self.requested_provider_capability = capability;
        result
    }

    /// Handle a request to connect as a consumer from the policy
    pub fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        let result = if self.psu_state == PsuState::Idle {
            Ok(())
        } else {
            Err(Error::InvalidState(&[StateKind::Idle], self.psu_state.kind()))
        };
        self.psu_state = PsuState::ConnectedConsumer(capability);
        result
    }

    /// Handle a request to connect as a provider from the policy
    pub fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        let result = if matches!(self.psu_state, PsuState::Idle | PsuState::ConnectedProvider(_)) {
            Ok(())
        } else {
            Err(Error::InvalidState(
                &[StateKind::Idle, StateKind::ConnectedProvider],
                self.psu_state.kind(),
            ))
        };
        self.psu_state = PsuState::ConnectedProvider(capability);
        result
    }

    /// Returns the current provider capability if the PSU is connected as a provider
    pub fn connected_provider_capability(&self) -> Option<ProviderPowerCapability> {
        match self.psu_state {
            PsuState::ConnectedProvider(capability) => Some(capability),
            _ => None,
        }
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
pub trait Psu {
    /// Disconnect power from this device
    fn disconnect(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to provide power to an external connection
    fn connect_provider(&mut self, capability: ProviderPowerCapability) -> impl Future<Output = Result<(), Error>>;
    /// Connect this device to consume power from an external connection
    fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> impl Future<Output = Result<(), Error>>;
    /// Return a mutable reference to the current PSU state
    fn state(&mut self) -> &mut State;
}

/// PSU registration struct
pub struct RegistrationEntry<'a, D: Lockable>
where
    D::Inner: Psu,
{
    /// Device ID
    pub id: DeviceId,
    /// Reference to hardware
    pub device: &'a D,
}

impl<'a, D: Lockable> RegistrationEntry<'a, D>
where
    D::Inner: Psu,
{
    /// Create a new device
    pub fn new(id: DeviceId, device: &'a D) -> Self {
        Self { id, device }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }
}
