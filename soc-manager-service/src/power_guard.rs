//! PowerGuard.
//!
//! This is intended to be used within `embedded-power-sequence` implementations for handling
//! rollback automatically while enabling/disabling power regulators.
//!
//! # Example
//!
//! ```rust,ignore
//! enum PowerSequenceError {
//!     Timeout,
//!     RegulatorFailure,
//! }
//!
//! impl<R: Regulator, I: InputPin + Wait> PowerSequence for SoC<R, I> {
//!     async fn power_on(&mut self) -> Result<(), PowerSequenceError> {
//!         let mut guard = power_guard::PowerGuard::<R, 3>::new();
//!
//!         // If any of these fail, the PowerGuard will be implicitly rolled back
//!         guard.execute(power_guard::Op::Enable(&mut self.regulator1)).await?;
//!         guard.execute(power_guard::Op::Enable(&mut self.regulator2)).await?;
//!         guard.execute(power_guard::Op::Enable(&mut self.regulator3)).await?;
//!
//!         // Typically at some point during sequencing we might wait for a "power good" pin to go high,
//!         // and if we timeout while waiting we can explicitly rollback the PowerGuard
//!         if with_timeout(Duration::from_millis(1000), self.pwr_good.wait_for_high()).await.is_err() {
//!             guard.rollback().await.map_err(|_| PowerSequenceError::RegulatorFailure)?;
//!             return Err(Error::Timeout);
//!         }
//!
//!         Ok(())
//!     }
//! }
//! ```
use embedded_regulator::Regulator;
use heapless::Vec;

/// PowerGuard error.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The PowerGuard is full and cannot accept more operations.
    Full,
    /// A regulator error occurred.
    RegulatorFailure,
    /// The PowerGuard is empty.
    Empty,
}

/// PowerGuard operation.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Op<'a, R> {
    /// Enable regulator.
    Enable(&'a mut R),
    /// Disable regulator.
    Disable(&'a mut R),
}

/// PowerGuard.
///
/// This represents a stack of operations on power regulators.
/// As operations are pushed to the stack, they are executed.
///
/// In the event of an error, operations are undone and removed from the PowerGuard in reverse order.
pub struct PowerGuard<'a, R: Regulator, const MAX_SIZE: usize> {
    stk: Vec<Op<'a, R>, MAX_SIZE>,
}

impl<'a, R: Regulator, const MAX_SIZE: usize> Default for PowerGuard<'a, R, MAX_SIZE> {
    fn default() -> Self {
        Self { stk: Vec::new() }
    }
}

impl<'a, R: Regulator, const MAX_SIZE: usize> PowerGuard<'a, R, MAX_SIZE> {
    /// Create a new PowerGuard instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rollback the PowerGuard. This will undo operations in reverse order of how they were entered.
    /// If successful, the PowerGuard will be empty upon return.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RegulatorFailure`] if a regulator error occurred during rollback.
    /// In this failure event, the PowerGuard may not be empty, and the failing regulator is NOT removed from the PowerGuard.
    pub async fn rollback(&mut self) -> Result<(), Error> {
        loop {
            match self.rollback_once().await {
                Ok(()) => continue,
                Err(Error::Empty) => return Ok(()),
                e @ Err(_) => return e,
            }
        }
    }

    /// Rollback only the single most recent operation in the PowerGuard.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Empty`] if the PowerGuard is empty.
    ///
    /// Returns [`Error::RegulatorFailure`] if a regulator error occurred during rollback.
    /// In this failure event, the failing regulator is NOT removed from the PowerGuard.
    pub async fn rollback_once(&mut self) -> Result<(), Error> {
        let res = match self.stk.last_mut() {
            Some(Op::Enable(r)) => r.disable().await,
            Some(Op::Disable(r)) => r.enable().await,
            None => return Err(Error::Empty),
        }
        .map_err(|_| Error::RegulatorFailure);

        if res.is_ok() {
            let _ = self.stk.pop();
        }

        res
    }

    /// Execute an operation on a regulator and add it to the PowerGuard.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Full`] if the PowerGuard is full. The PowerGuard is NOT rolled back in this case.
    ///
    /// Returns [`Error::RegulatorFailure`] if the regulator failed to perform the requested operation.
    /// In this failure event, the PowerGuard is rolled back, and the failing regulator is NOT added to the PowerGuard.
    /// During rollback a regulator may fail, in which case this error is also returned.
    pub async fn execute(&mut self, mut cmd: Op<'a, R>) -> Result<(), Error> {
        if self.stk.is_full() {
            return Err(Error::Full);
        }

        let res = match &mut cmd {
            Op::Enable(r) => r.enable().await,
            Op::Disable(r) => r.disable().await,
        };

        if res.is_ok() {
            // Note: This will always succeed since we checked the stack isn't full above
            let _ = self.stk.push(cmd);
            Ok(())
        } else {
            self.rollback().await?;
            Err(Error::RegulatorFailure)
        }
    }

    /// Pops the most recent regulator (if any) from the PowerGuard without attempting to roll back the operation.
    pub fn pop(&mut self) -> Option<&mut R> {
        match self.stk.pop() {
            Some(Op::Enable(r)) | Some(Op::Disable(r)) => Some(r),
            None => None,
        }
    }

    /// Clears the PowerGuard of all operations.
    pub fn clear(&mut self) {
        self.stk.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use embedded_regulator::{ErrorKind, ErrorType};

    /// A mock regulator that tracks enable/disable state.
    struct MockReg {
        enabled: bool,
        always_fail: bool,
    }

    impl MockReg {
        fn new(always_fail: bool) -> Self {
            Self {
                enabled: false,
                always_fail,
            }
        }
    }

    impl ErrorType for MockReg {
        type Error = ErrorKind;
    }

    impl Regulator for MockReg {
        async fn enable(&mut self) -> Result<(), Self::Error> {
            if self.always_fail {
                Err(ErrorKind::Other)
            } else {
                self.enabled = true;
                Ok(())
            }
        }

        async fn disable(&mut self) -> Result<(), Self::Error> {
            if self.always_fail {
                Err(ErrorKind::Other)
            } else {
                self.enabled = false;
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_execute() {
        let mut r1 = MockReg::new(false);
        let mut r2 = MockReg::new(false);

        {
            let mut guard = PowerGuard::<MockReg, 2>::new();
            guard.execute(Op::Enable(&mut r1)).await.unwrap();
            guard.execute(Op::Enable(&mut r2)).await.unwrap();
        }

        // Verify we can execute operations on the regulators
        assert!(r1.enabled);
        assert!(r2.enabled);
    }

    #[tokio::test]
    async fn test_execute_and_rollback() {
        let mut r1 = MockReg::new(false);
        let mut r2 = MockReg::new(false);

        {
            let mut guard = PowerGuard::<MockReg, 2>::new();
            guard.execute(Op::Enable(&mut r1)).await.unwrap();
            guard.execute(Op::Enable(&mut r2)).await.unwrap();
            guard.rollback().await.unwrap();
        }

        // Verify we can rollback operations on the regulators
        assert!(!r1.enabled);
        assert!(!r2.enabled);
    }

    #[tokio::test]
    async fn test_full() {
        let mut r1 = MockReg::new(false);
        let mut r2 = MockReg::new(false);
        let mut r3 = MockReg::new(false);

        let mut guard = PowerGuard::<MockReg, 2>::new();
        guard.execute(Op::Enable(&mut r1)).await.unwrap();
        guard.execute(Op::Enable(&mut r2)).await.unwrap();

        // Verify we can't add a third regulator
        assert!(matches!(guard.execute(Op::Enable(&mut r3)).await, Err(Error::Full)));
    }

    #[tokio::test]
    async fn test_execute_and_rollback_once() {
        let mut r1 = MockReg::new(false);
        let mut r2 = MockReg::new(false);

        {
            let mut guard = PowerGuard::<MockReg, 2>::new();
            guard.execute(Op::Enable(&mut r1)).await.unwrap();
            guard.execute(Op::Enable(&mut r2)).await.unwrap();
            guard.rollback_once().await.unwrap();
        }

        // Verify only r2 is rolled back (disabled) and r1 remains enabled
        assert!(r1.enabled);
        assert!(!r2.enabled);
    }

    #[tokio::test]
    async fn test_execute_failure() {
        let mut r1 = MockReg::new(true);

        {
            let mut guard = PowerGuard::<MockReg, 1>::new();
            assert!(matches!(
                guard.execute(Op::Enable(&mut r1)).await,
                Err(Error::RegulatorFailure)
            ));
        }

        // Verify r1 remains disabled after operation failure
        assert!(!r1.enabled);
    }
}
