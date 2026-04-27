//! Charger events, state machine, and trait

use crate::capability::ConsumerPowerCapability;
use core::{convert::Infallible, future::Future};

pub mod event;
/// Mock software representation of a charger
pub mod mock;
#[cfg(test)]
mod tests;

pub use event::{Event, EventData, PsuState};

/// Charger Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChargerId(pub u8);

/// Charger state errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum ChargerError {
    /// Charger received command in an invalid state
    InvalidState(InternalState),
    /// Charger hardware timed out responding
    Timeout,
    /// Charger underlying bus error
    BusError,
    /// Charger received an unknown event
    UnknownEvent,
}

impl From<ChargerError> for crate::psu::Error {
    fn from(value: ChargerError) -> Self {
        Self::Charger(value)
    }
}

impl From<Infallible> for ChargerError {
    fn from(_value: Infallible) -> Self {
        Self::BusError
    }
}

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InternalState {
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
pub struct State {
    /// Charger device state
    state: InternalState,
    /// Current charger capability
    capability: Option<ConsumerPowerCapability>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            state: InternalState::Unpowered,
            capability: None,
        }
    }
}

impl State {
    /// Returns a reference to the current internal charger state.
    pub fn internal_state(&self) -> &InternalState {
        &self.state
    }

    /// Returns a reference to the current cached power capability, if any.
    pub fn capability(&self) -> &Option<ConsumerPowerCapability> {
        &self.capability
    }

    /// Handle charger initialization completing. Transitions from `Powered(Init)` to
    /// `Powered(PsuAttached)` or `Powered(PsuDetached)` based on PSU state.
    ///
    /// Returns `Err` if not in `Powered(Init)`.
    pub fn on_initialized(&mut self, psu_state: PsuState) -> Result<(), ChargerError> {
        match self.state {
            InternalState::Powered(PoweredSubstate::Init) => {
                self.state = match psu_state {
                    PsuState::Attached => InternalState::Powered(PoweredSubstate::PsuAttached),
                    PsuState::Detached => InternalState::Powered(PoweredSubstate::PsuDetached),
                };
                Ok(())
            }
            other => Err(ChargerError::InvalidState(other)),
        }
    }

    /// Handle a PSU state change event. Transitions between `Powered(PsuAttached)` and
    /// `Powered(PsuDetached)`.
    ///
    /// Returns `Err` if not in `Powered(PsuAttached)` or `Powered(PsuDetached)`.
    pub fn on_psu_state_change(&mut self, psu_state: PsuState) -> Result<(), ChargerError> {
        match self.state {
            InternalState::Powered(PoweredSubstate::PsuAttached) => {
                if psu_state == PsuState::Detached {
                    self.state = InternalState::Powered(PoweredSubstate::PsuDetached);
                }
                Ok(())
            }
            InternalState::Powered(PoweredSubstate::PsuDetached) => {
                if psu_state == PsuState::Attached {
                    self.state = InternalState::Powered(PoweredSubstate::PsuAttached);
                }
                Ok(())
            }
            other => Err(ChargerError::InvalidState(other)),
        }
    }

    /// Handle a communication timeout. Transitions to `Unpowered` and clears the cached capability.
    pub fn on_timeout(&mut self) {
        self.state = InternalState::Unpowered;
        self.capability = None;
    }

    /// Transition after a successful check-ready response.
    ///
    /// If currently unpowered, moves to `Powered(Init)` and clears capability.
    /// If already powered, this is a no-op.
    pub fn on_ready_success(&mut self) {
        if self.state == InternalState::Unpowered {
            self.state = InternalState::Powered(PoweredSubstate::Init);
            self.capability = None;
        }
    }

    /// Transition after a failed check-ready response.
    ///
    /// If currently powered, moves to `Unpowered`. Capability is preserved for diagnostics.
    /// If already unpowered, this is a no-op.
    pub fn on_ready_failure(&mut self) {
        if matches!(self.state, InternalState::Powered(_)) {
            self.state = InternalState::Unpowered;
        }
    }

    /// Cache a new capability from a policy configuration attach.
    /// Does not change the charger state.
    pub fn on_policy_attach(&mut self, capability: ConsumerPowerCapability) {
        self.capability = Some(capability);
    }

    /// Clear the cached capability after a policy configuration detach.
    /// Does not change the charger state.
    pub fn on_policy_detach(&mut self) {
        self.capability = None;
    }

    /// Returns `true` if the charger is in the `Unpowered` state.
    pub fn is_unpowered(&self) -> bool {
        self.state == InternalState::Unpowered
    }
}

/// Charger controller trait that devices must implement to use the power policy service.
pub trait Charger: embedded_batteries_async::charger::Charger {
    /// Type of error returned by the bus
    type ChargerError: Into<ChargerError> + embedded_batteries_async::charger::Error;

    /// Initialize charger hardware, after this returns the charger should be ready to charge
    fn init_charger(&mut self) -> impl Future<Output = Result<PsuState, Self::ChargerError>>;
    /// Called after power policy attaches to a power port.
    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargerError>>;
    /// Called after power policy detaches from a power port, either to switch consumers,
    /// or because PSU was disconnected.
    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargerError>>;
    /// Upon successful return of this method, the charger is assumed to be powered and ready to communicate,
    /// transitioning state from unpowered to powered.
    fn is_ready(&mut self) -> impl Future<Output = Result<(), Self::ChargerError>> {
        core::future::ready(Ok(()))
    }
    /// Return an immutable reference to the current charger state
    fn state(&self) -> &State;
    /// Return a mutable reference to the current charger state
    fn state_mut(&mut self) -> &mut State;
}
