//! This module contains types for a very basic firmware update interface.

use embedded_services::named::Named;

/// Basic FW update error type
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The operation is not valid during a FW update
    UpdateInProgress,
    /// This operation is only valid when a FW update is in progress
    NeedsActiveUpdate,
    /// Invalid address
    InvalidAddress(usize),
    /// The firmware content is invalid
    InvalidContent,
    /// The requested operation timed out
    Timeout,
    /// The device is busy
    Busy,
    /// Bus error
    Bus,
    /// Unspecified failure
    Failed,
}

/// Basic FW update trait
///
/// This is for devices that don't need to expose multiple banks and can support
/// a FW update done through a few operations. Write only.
pub trait FwUpdate: Named {
    /// Get current FW version
    fn get_active_fw_version(&mut self) -> impl Future<Output = Result<u32, Error>>;
    /// Start a firmware update
    fn start_fw_update(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Abort a firmware update
    fn abort_fw_update(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Finalize a firmware update
    fn finalize_fw_update(&mut self) -> impl Future<Output = Result<(), Error>>;
    /// Write firmware update contents
    fn write_fw_contents(&mut self, offset: usize, data: &[u8]) -> impl Future<Output = Result<(), Error>>;
}
