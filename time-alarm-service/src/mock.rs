#![allow(dead_code)] // We have some functionality in these mocks that isn't used yet but will be in future tests.

use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClock, DatetimeClockError};

// Used for `cargo test` runs in an std environment
#[cfg(test)]
fn now_seconds() -> u64 {
    // Panic safety: Only used in tests so panicking is acceptable here
    #[allow(clippy::expect_used)]
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("System clock was adjusted during test")
        .as_secs()
}

// Allows us to use this mock in no_std contexts
// Note: `get_current_datetime` will always reflect time as starting from the beginning
// of UNIX time (1970), and not current wall-clock time. This is sufficient for its use case
// since it provides a consistent baseline and allows time to advance, but is something to be aware of.
#[cfg(not(test))]
fn now_seconds() -> u64 {
    embassy_time::Instant::now().as_secs()
}

pub enum MockDatetimeClock {
    Running { seconds_offset: i64 },
    Paused { frozen_time: Datetime },
}

impl MockDatetimeClock {
    /// New `MockDatetimeClock` in which time is advancing.
    pub fn new_running() -> Self {
        Self::Running { seconds_offset: 0 }
    }

    /// New `MockDatetimeClock` in which time is paused.
    pub fn new_paused() -> Self {
        Self::Paused {
            frozen_time: Datetime::from_unix_time_seconds(now_seconds()),
        }
    }

    /// Stop time from advancing.
    pub fn pause(&mut self) {
        if let Self::Running { .. } = self {
            *self = MockDatetimeClock::Paused {
                // Panic safety: Mocks aren't used in production code, so panicking is acceptable here
                #[allow(clippy::unwrap_used)]
                frozen_time: self.get_current_datetime().unwrap(),
            };
        }
    }

    /// Resume time advancing.
    pub fn resume(&mut self) {
        if let Self::Paused { frozen_time } = self {
            let target_secs = frozen_time.to_unix_time_seconds() as i64;
            *self = MockDatetimeClock::Running {
                seconds_offset: target_secs - now_seconds() as i64,
            };
        }
    }
}

impl DatetimeClock for MockDatetimeClock {
    fn get_current_datetime(&self) -> Result<Datetime, DatetimeClockError> {
        match self {
            MockDatetimeClock::Paused { frozen_time } => Ok(*frozen_time),
            MockDatetimeClock::Running { seconds_offset } => {
                let adjusted_seconds = now_seconds() as i64 + seconds_offset;
                Ok(Datetime::from_unix_time_seconds(adjusted_seconds as u64))
            }
        }
    }

    fn set_current_datetime(&mut self, datetime: &Datetime) -> Result<(), DatetimeClockError> {
        match self {
            MockDatetimeClock::Paused { .. } => {
                *self = MockDatetimeClock::Paused { frozen_time: *datetime };
                Ok(())
            }
            MockDatetimeClock::Running { .. } => {
                let target_secs = datetime.to_unix_time_seconds() as i64;
                *self = MockDatetimeClock::Running {
                    seconds_offset: target_secs - now_seconds() as i64,
                };
                Ok(())
            }
        }
    }

    fn max_resolution_hz(&self) -> u32 {
        1
    }
}

pub struct MockNvramStorage<'a> {
    value: u32,
    _phantom: core::marker::PhantomData<&'a ()>,
}

impl<'a> MockNvramStorage<'a> {
    pub fn new(initial_value: u32) -> Self {
        Self {
            value: initial_value,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a> NvramStorage<'a, u32> for MockNvramStorage<'a> {
    fn read(&self) -> u32 {
        self.value
    }

    fn write(&mut self, value: u32) {
        self.value = value;
    }
}
