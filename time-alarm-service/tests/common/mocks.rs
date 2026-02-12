#![allow(dead_code)] // We have some functionality in these mocks that isn't used yet but will be in future tests.

use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClock, DatetimeClockError};
use std::time::SystemTime;

pub(crate) enum MockDatetimeClock {
    Running { seconds_offset_from_system_time: i64 },
    Paused { frozen_time: Datetime },
}

impl MockDatetimeClock {
    pub(crate) fn new_running() -> Self {
        Self::Running {
            seconds_offset_from_system_time: 0,
        }
    }

    pub(crate) fn new_paused() -> Self {
        Self::Paused {
            frozen_time: Datetime::from_unix_time_seconds(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("System clock was adjusted during test")
                    .as_secs(),
            ),
        }
    }

    /// Stop time from advancing.
    pub(crate) fn pause(&mut self) {
        if let Self::Running { .. } = self {
            *self = MockDatetimeClock::Paused {
                frozen_time: self.get_current_datetime().unwrap(),
            };
        }
    }

    /// Resume time advancing.
    pub(crate) fn resume(&mut self) {
        if let Self::Paused { frozen_time } = self {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("System clock was adjusted during test");
            let target_secs = frozen_time.to_unix_time_seconds() as i64;
            let adjusted_seconds = now.as_secs() as i64;
            *self = MockDatetimeClock::Running {
                seconds_offset_from_system_time: target_secs - adjusted_seconds,
            };
        }
    }
}

impl DatetimeClock for MockDatetimeClock {
    fn get_current_datetime(&self) -> Result<Datetime, DatetimeClockError> {
        match self {
            MockDatetimeClock::Paused { frozen_time } => Ok(*frozen_time),
            MockDatetimeClock::Running {
                seconds_offset_from_system_time,
            } => {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("System clock was adjusted during test");
                let adjusted_seconds = now.as_secs() as i64 + seconds_offset_from_system_time;
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
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("System clock was adjusted during test");

                *self = MockDatetimeClock::Running {
                    seconds_offset_from_system_time: target_secs - (now.as_secs() as i64),
                };
                Ok(())
            }
        }
    }

    fn max_resolution_hz(&self) -> u32 {
        1
    }
}

pub(crate) struct MockNvramStorage<'a> {
    value: u32,
    _phantom: core::marker::PhantomData<&'a ()>,
}

impl<'a> MockNvramStorage<'a> {
    pub(crate) fn new(initial_value: u32) -> Self {
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
