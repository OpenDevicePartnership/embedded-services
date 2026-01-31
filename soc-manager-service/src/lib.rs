//! SoC manager service.
#![no_std]

pub mod power_guard;

use embassy_sync::mutex::Mutex;
use embassy_sync::watch::{Receiver, Watch};
use embedded_power_sequence::PowerSequence;
use embedded_services::GlobalRawMutex;

/// SoC manager service error.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Unspecified error, likely some invariant was violated.
    Other,
    /// A power sequence error occurred.
    PowerSequence,
    /// An invalid power state transition was requested.
    InvalidStateTransition,
    /// No more power state listeners are available.
    ListenersNotAvailable,
}

/// An ACPI power state.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerState {
    /// Working state.
    S0,
    /// Modern standby state.
    S0ix,
    /// Sleep state.
    S3,
    /// Hibernate state.
    S4,
    /// Soft off state.
    S5,
}

/// A power state listener struct.
pub struct PowerStateListener<'a, const MAX_LISTENERS: usize> {
    rx: Receiver<'a, GlobalRawMutex, PowerState, MAX_LISTENERS>,
}

impl<'a, const MAX_LISTENERS: usize> PowerStateListener<'a, MAX_LISTENERS> {
    /// Waits for any power state change, returning the new power state.
    pub fn wait_state_change(&mut self) -> impl Future<Output = PowerState> {
        self.rx.changed()
    }

    /// Waits for a transition to a specific power state.
    pub async fn wait_for_state(&mut self, power_state: PowerState) {
        self.rx.changed_and(|p| *p == power_state).await;
    }

    /// Returns the current power state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Other`] if the power state is uninitialized.
    pub fn current_state(&mut self) -> Result<PowerState, Error> {
        self.rx.try_get().ok_or(Error::Other)
    }
}

/// SoC manager.
pub struct SocManager<T: PowerSequence, const MAX_LISTENERS: usize> {
    soc: Mutex<GlobalRawMutex, T>,
    power_state: Watch<GlobalRawMutex, PowerState, MAX_LISTENERS>,
}

impl<T: PowerSequence, const MAX_LISTENERS: usize> SocManager<T, MAX_LISTENERS> {
    /// Creates a new SoC manager instance.
    ///
    /// The `initial_state` should capture the power state the SoC is ALREADY in, not the desired state
    /// to transition to on initilization.
    ///
    /// This will usually be [`PowerState::S5`] (powered off) but not always.
    pub fn new(soc: T, initial_state: PowerState) -> Self {
        let soc_manager = Self {
            soc: Mutex::new(soc),
            power_state: Watch::new(),
        };

        soc_manager.power_state.sender().send(initial_state);
        soc_manager
    }

    /// Creates a new power state listener.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ListenersNotAvailable`] if `MAX_LISTENERS` or greater are already in use.
    pub fn new_pwr_listener(&self) -> Result<PowerStateListener<'_, MAX_LISTENERS>, Error> {
        Ok(PowerStateListener {
            rx: self.power_state.receiver().ok_or(Error::InvalidStateTransition)?,
        })
    }

    /// Returns the current power state.
    ///
    /// This method is also available on `PowerStateListener`.
    pub fn current_state(&self) -> Result<PowerState, Error> {
        self.power_state.try_get().ok_or(Error::Other)
    }

    /// Sets the current power state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PowerSequence`] if an error is encountered while transitioning power state.
    ///
    /// Returns [`Error::InvalidStateTransition`] if the requested state is not valid based on current state.
    pub async fn set_power_state(&self, state: PowerState) -> Result<(), Error> {
        // Revisit: Check with other services to see if we are too hot or don't have enough power for requested transition
        // Need to think more about how that will look though
        let cur_state = self.power_state.try_get().ok_or(Error::Other)?;
        let mut soc = self.soc.lock().await;
        match (cur_state, state) {
            // Any sleeping state must first transition to S0 before we can transition to another state
            (PowerState::S0ix, PowerState::S0) => soc.wake_up().await,
            (PowerState::S3, PowerState::S0) => soc.resume().await,
            (PowerState::S4, PowerState::S0) => soc.activate().await,
            (PowerState::S5, PowerState::S0) => soc.power_on().await,

            // S0 can then transition to any sleep state
            (PowerState::S0, PowerState::S0ix) => soc.idle().await,
            (PowerState::S0, PowerState::S3) => soc.suspend().await,
            (PowerState::S0, PowerState::S4) => soc.hibernate().await,
            (PowerState::S0, PowerState::S5) => soc.power_off().await,

            // Anything else is an invalid transition
            _ => return Err(Error::InvalidStateTransition),
        }
        .map_err(|_| Error::PowerSequence)?;

        self.power_state.sender().send(state);
        Ok(())
    }
}
