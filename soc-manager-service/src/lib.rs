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
    /// to transition to on initialization.
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
            rx: self.power_state.receiver().ok_or(Error::ListenersNotAvailable)?,
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
        let mut soc = self.soc.lock().await;
        let cur_state = self.power_state.try_get().ok_or(Error::Other)?;

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use embedded_power_sequence::{ErrorKind, ErrorType};

    /// A mock SoC that always succeeds power transitions.
    struct MockSoc;

    impl ErrorType for MockSoc {
        type Error = ErrorKind;
    }

    impl PowerSequence for MockSoc {
        async fn power_on(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn pre_power_on(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn post_power_on(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn power_off(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn pre_power_off(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn post_power_off(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let sm = SocManager::<MockSoc, 2>::new(MockSoc, PowerState::S5);

        // Verify that we can't directly transition to a sleeping state (S3)
        assert!(matches!(
            sm.set_power_state(PowerState::S3).await,
            Err(Error::InvalidStateTransition)
        ));

        // Verify state remains unchanged
        assert!(sm.current_state().unwrap() == PowerState::S5);

        // Verify we can transition to S0
        assert!(sm.set_power_state(PowerState::S0).await.is_ok());

        // Verify state has changed to S0
        assert!(sm.current_state().unwrap() == PowerState::S0);

        // Verify we can then transition to a sleeping state (S3)
        assert!(sm.set_power_state(PowerState::S3).await.is_ok());

        // Verify state has changed to S3
        assert!(sm.current_state().unwrap() == PowerState::S3);
    }

    #[tokio::test]
    async fn test_power_state_listener() {
        let sm = SocManager::<MockSoc, 2>::new(MockSoc, PowerState::S5);

        // Create two listeners
        let mut l1 = sm.new_pwr_listener().unwrap();
        let mut l2 = sm.new_pwr_listener().unwrap();

        // Verify we can't create a third
        assert!(matches!(sm.new_pwr_listener(), Err(Error::ListenersNotAvailable)));

        // Verify listeners can read initial state
        assert_eq!(l1.current_state().unwrap(), PowerState::S5);
        assert_eq!(l2.current_state().unwrap(), PowerState::S5);

        // Verify listeners can read updated state
        sm.set_power_state(PowerState::S0).await.unwrap();
        assert_eq!(l1.current_state().unwrap(), PowerState::S0);
        assert_eq!(l2.current_state().unwrap(), PowerState::S0);

        // Verify listeners can wait for state changes
        sm.set_power_state(PowerState::S0ix).await.unwrap();
        assert_eq!(l1.wait_state_change().await, PowerState::S0ix);
        assert_eq!(l2.wait_state_change().await, PowerState::S0ix);

        // Verify listeners can wait for specific state change
        sm.set_power_state(PowerState::S0).await.unwrap();
        l1.wait_for_state(PowerState::S0).await;
        l2.wait_for_state(PowerState::S0).await;
        // If we got here then they successfully waited for S0
    }
}
