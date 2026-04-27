//! Configuration structs

use embassy_time::Duration;

/// Base interval for checking for FW update timeouts and recovery attempts
pub const DEFAULT_FW_UPDATE_TICK_INTERVAL: Duration = Duration::from_secs(5);
/// Default number of ticks before we consider a firmware update to have timed out
/// 300 seconds at 5 seconds per tick
pub const DEFAULT_FW_UPDATE_TIMEOUT_TICKS: u32 = 60;

/// Config values for FW update recovery
#[derive(Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Recovery {
    /// Interval between recovery ticks
    pub tick_interval: Duration,
    /// Timeout (in recovery ticks) before we assume the update has failed.
    pub update_timeout_ticks: u32,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            tick_interval: DEFAULT_FW_UPDATE_TICK_INTERVAL,
            update_timeout_ticks: DEFAULT_FW_UPDATE_TIMEOUT_TICKS,
        }
    }
}

/// Configuration for [`crate::basic::Updater`]
#[derive(Copy, Clone, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Updater {
    /// Recovery configuration for the updater
    pub recovery: Recovery,
}

/// Configuration for [`crate::basic::event_receiver::EventReceiver`]
#[derive(Copy, Clone, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct EventReceiver {
    /// Recovery configuration for the event receiver
    pub recovery: Recovery,
}
