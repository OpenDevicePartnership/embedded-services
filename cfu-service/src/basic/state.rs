use embassy_time::{Duration, Instant};

/// Current state of the firmware update process
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FwUpdateState {
    /// None in progress
    #[default]
    Idle,
    /// Firmware update in progress.
    /// Integer is number of recovery ticks that have occurred since the start of the update.
    InProgress(u32),
    /// Firmware update has failed and the device is in an unknown state
    Recovery,
}

/// State shared between [`crate::basic::event_receiver::EventReceiver`] and [`crate::basic::Updater`]
#[derive(Clone, Copy)]
pub struct SharedState {
    /// Current update state
    pub(super) fw_update_state: FwUpdateState,
    /// Next recovery tick
    pub(super) next_recovery_tick: Instant,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            fw_update_state: FwUpdateState::Idle,
            next_recovery_tick: Instant::MAX,
        }
    }

    pub(super) fn enter_idle(&mut self) {
        self.fw_update_state = FwUpdateState::Idle;
        self.next_recovery_tick = Instant::MAX;
    }

    pub(super) fn enter_in_progress(&mut self, next_recovery_tick: Duration) {
        self.fw_update_state = FwUpdateState::InProgress(0);
        self.next_recovery_tick = Instant::now() + next_recovery_tick;
    }

    pub(super) fn enter_recovery(&mut self) {
        self.fw_update_state = FwUpdateState::Recovery;
        if self.next_recovery_tick == Instant::MAX {
            self.next_recovery_tick = Instant::now();
        }
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}
