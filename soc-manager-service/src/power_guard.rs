//! PowerGuard.
//!
//! This is intended to be used within `embedded-power-sequence` implementations for handling
//! rollback automatically while enabling/disabling power regulators.
//!
//! # Example
//!
//! ```rust,ignore
//! impl<R: Regulator, I: InputPin + Wait> PowerSequence for SoC<R, I> {
//!     async fn power_on(&mut self) -> Result<(), Error> {
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
//!             guard.rollback().await?;
//!         }
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
    /// A regulator error occurred during rollback.
    RollbackFailure,
    /// A regulator error occurred while pushing an operation into the PowerGuard.
    OpFailure,
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
    /// Returns [`Error::RollbackFailure`] if a regulator error occurred during rollback.
    /// In this failure event, the PowerGuard may not be empty.
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
    /// Returns [`Error::RollbackFailure`] if a regulator error occurred during rollback.
    pub async fn rollback_once(&mut self) -> Result<(), Error> {
        match self.stk.pop() {
            Some(Op::Enable(r)) => r.disable().await,
            Some(Op::Disable(r)) => r.enable().await,
            None => return Err(Error::Empty),
        }
        .map_err(|_| Error::RollbackFailure)
    }

    /// Execute an operation on a wrapped power regulator.
    /// If the operation fails, the PowerGuard will be automatically rolled back.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Full`] if the PowerGuard is full. The PowerGuard is NOT rolled back in this case.
    ///
    /// Returns [`Error::OpFailure`] if the operation failed but rollback was successful.
    ///
    /// Returns [`Error::RollbackFailure`] if the operation failed and rollback failed as well.
    pub async fn execute(&mut self, mut cmd: Op<'a, R>) -> Result<(), Error> {
        if self.stk.is_full() {
            return Err(Error::Full);
        }

        let res = match &mut cmd {
            Op::Enable(r) => r.enable().await,
            Op::Disable(r) => r.disable().await,
        };

        if res.is_ok() {
            let _ = self.stk.push(cmd);
            Ok(())
        } else {
            self.rollback().await?;
            Err(Error::OpFailure)
        }
    }

    /// Clears the PowerGuard of all operations.
    /// Thus, they will not be included in future rollbacks.
    pub fn clear(&mut self) {
        self.stk.clear();
    }
}
