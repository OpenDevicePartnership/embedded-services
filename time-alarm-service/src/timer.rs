use crate::{AlarmExpiredWakePolicy, ClockState, TimerStatus};
use core::cell::RefCell;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::Mutex, signal::Signal};
use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClockError};
use embedded_services::{GlobalRawMutex, error};

/// Represents where in the timer lifecycle the current timer is
#[derive(Copy, Clone, Debug, PartialEq)]
enum WakeState {
    /// Timer is not active
    Clear,
    /// Timer is active and programmed with the original expiration time
    Armed,
    /// Timer is active but expired when on the wrong power source
    /// Includes the time at which we started running down the policy delay and the number of seconds that had already elapsed on the policy delay when we started waiting
    ExpiredWaitingForPolicyDelay(Datetime, u32),
    /// Timer is active and waiting for power source to be consistent with the timer type.
    /// Includes the number of seconds that we've spent in the ExpiredWaitingForPolicyDelay state for so far.
    ExpiredWaitingForPowerSource(u32),
    /// Expired while the policy was set to NEVER, so the timer is effectively dead until reprogrammed
    ExpiredOrphaned,
}

mod persistent_storage {
    use crate::{AlarmExpiredWakePolicy, Datetime};
    use embedded_mcu_hal::NvramStorage;

    pub struct PersistentStorage<'hw> {
        /// When the timer is programmed to expire, or None if the timer is not set
        /// This can't be part of the wake_state because we need to be able to report its value for _CWS even when the timer has expired and
        /// we're handling the power source policy.
        expiration_time_storage: &'hw mut dyn NvramStorage<'hw, u32>,

        // Persistent storage for the AlarmExpiredWakePolicy
        wake_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
    }

    impl<'hw> PersistentStorage<'hw> {
        pub fn new(
            expiration_time_storage: &'hw mut dyn NvramStorage<'hw, u32>,
            wake_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        ) -> Self {
            Self {
                expiration_time_storage,
                wake_policy_storage,
            }
        }

        const NO_EXPIRATION_TIME: u32 = u32::MAX;

        pub fn get_timer_wake_policy(&self) -> AlarmExpiredWakePolicy {
            AlarmExpiredWakePolicy(self.wake_policy_storage.read())
        }

        pub fn set_timer_wake_policy(&mut self, wake_policy: AlarmExpiredWakePolicy) {
            self.wake_policy_storage.write(wake_policy.0);
        }

        pub fn get_expiration_time(&self) -> Option<Datetime> {
            match self.expiration_time_storage.read() {
                Self::NO_EXPIRATION_TIME => None,
                secs => Some(Datetime::from_unix_time_seconds(secs.into())),
            }
        }

        pub fn set_expiration_time(&mut self, expiration_time: Option<Datetime>) {
            match expiration_time {
                Some(dt) => {
                    // This won't overflow until 2106, which is acceptable for our use case.
                    self.expiration_time_storage.write(dt.to_unix_time_seconds() as u32);
                }
                None => {
                    self.expiration_time_storage.write(Self::NO_EXPIRATION_TIME);
                }
            }
        }
    }
}
use persistent_storage::PersistentStorage;

struct TimerState<'hw> {
    persistent_storage: PersistentStorage<'hw>,

    wake_state: WakeState,

    timer_status: TimerStatus,

    // Whether or not this timer is currently active (i.e. the system is on the power source this timer manages)
    // Even if it's not active, it still counts down if it's programmed - it just won't trigger a wake event if it expires while inactive.
    is_active: bool,
}

pub(crate) struct Timer<'hw> {
    timer_state: Mutex<GlobalRawMutex, RefCell<TimerState<'hw>>>,

    timer_signal: Signal<GlobalRawMutex, Option<u32>>,
}

impl<'hw> Timer<'hw> {
    pub fn new(
        expiration_time_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        wake_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
    ) -> Self {
        Self {
            timer_state: Mutex::new(RefCell::new(TimerState {
                persistent_storage: PersistentStorage::new(expiration_time_storage, wake_policy_storage),
                wake_state: WakeState::Clear,
                timer_status: Default::default(),
                is_active: false,
            })),
            timer_signal: Signal::new(),
        }
    }

    pub fn start(
        &self,
        clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>,
        active: bool,
    ) -> Result<(), DatetimeClockError> {
        self.set_timer_wake_policy(
            clock_state,
            self.timer_state
                .lock(|timer_state| timer_state.borrow().persistent_storage.get_timer_wake_policy()),
        )?;

        self.set_expiration_time(
            clock_state,
            self.timer_state
                .lock(|timer_state| timer_state.borrow().persistent_storage.get_expiration_time()),
        )?;

        self.set_active(clock_state, active);

        Ok(())
    }

    pub fn get_wake_status(&self) -> TimerStatus {
        self.timer_state.lock(|timer_state| {
            let timer_state = timer_state.borrow();
            timer_state.timer_status
        })
    }

    pub fn clear_wake_status(&self) {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();
            timer_state.timer_status = Default::default();
        });
    }

    pub fn get_timer_wake_policy(&self) -> AlarmExpiredWakePolicy {
        self.timer_state
            .lock(|timer_state| timer_state.borrow().persistent_storage.get_timer_wake_policy())
    }

    pub fn set_timer_wake_policy(
        &self,
        clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>,
        wake_policy: AlarmExpiredWakePolicy,
    ) -> Result<(), DatetimeClockError> {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();
            if let WakeState::ExpiredWaitingForPolicyDelay(_, _) = timer_state.wake_state {
                timer_state.wake_state =
                    WakeState::ExpiredWaitingForPolicyDelay(Self::get_current_datetime(clock_state)?, 0);
                self.timer_signal.signal(Some(wake_policy.0));
            }

            timer_state.persistent_storage.set_timer_wake_policy(wake_policy);

            Ok(())
        })
    }

    pub fn set_expiration_time(
        &self,
        clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>,
        expiration_time: Option<Datetime>,
    ) -> Result<(), DatetimeClockError> {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            // Per ACPI 6.4 section 9.18.1: "The status of wake timers can be reset by setting the wake alarm".
            timer_state.timer_status = Default::default();

            match expiration_time {
                Some(dt) => {
                    // Note: If the expiration time was in the past, this will immediately trigger the timer to expire.
                    self.timer_signal.signal(Some(
                        dt.to_unix_time_seconds()
                            .saturating_sub(Self::get_current_datetime(clock_state)?.to_unix_time_seconds())
                            as u32, // The ACPI spec doesn't provide a facility to program a timer more than u32::MAX seconds in the future, so this cast is safe
                    ));

                    timer_state.persistent_storage.set_expiration_time(expiration_time);
                    timer_state.wake_state = WakeState::Armed;
                }
                None => self.clear_expiration_time(&mut timer_state),
            }

            Ok(())
        })
    }

    pub fn get_expiration_time(&self) -> Option<Datetime> {
        self.timer_state
            .lock(|timer_state| timer_state.borrow().persistent_storage.get_expiration_time())
    }

    pub fn set_active(&self, clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>, is_active: bool) {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            let was_active = timer_state.is_active;
            timer_state.is_active = is_active;

            if was_active == is_active {
                return; // No change
            }

            if !was_active {
                if let WakeState::ExpiredWaitingForPowerSource(seconds_already_elapsed) = timer_state.wake_state {
                    match Self::get_current_datetime(clock_state) {
                        Ok(now) => {
                            timer_state.wake_state =
                                WakeState::ExpiredWaitingForPolicyDelay(now, seconds_already_elapsed);
                            self.timer_signal.signal(Some(
                                timer_state
                                    .persistent_storage
                                    .get_timer_wake_policy()
                                    .0
                                    .saturating_sub(seconds_already_elapsed),
                            ));
                        }
                        Err(_) => {
                            // This should never happen, because it means the clock is not working after we've successfully initialized (which
                            // requires the clock to be working).
                            // If it does, though, we don't have a way to communicate failure to the host PC at this point, so we'll just
                            // forego the power source policy and wake the device immediately.
                            error!(
                                "[Time/Alarm] Failed to get current datetime when transitioning timer to active state"
                            );
                            timer_state.wake_state = WakeState::Armed;
                            self.timer_signal.signal(Some(0));
                        }
                    }
                }
            } else if let WakeState::ExpiredWaitingForPolicyDelay(wait_start_time, seconds_elapsed_before_wait) =
                timer_state.wake_state
            {
                let total_seconds_elapsed_on_policy_delay = match Self::get_current_datetime(clock_state) {
                    Ok(now) => {
                        seconds_elapsed_before_wait
                            + (now
                                .to_unix_time_seconds()
                                .saturating_sub(wait_start_time.to_unix_time_seconds())
                                as u32) // The ACPI spec expresses timeouts in terms of u32s - it's impossible to schedule a timer u32::MAX seconds in the future
                    }
                    Err(_) => {
                        // This should never happen, because it means the clock is not working after we've successfully initialized (which
                        // requires the clock to be working).
                        // If it does, though, we don't have a way to communicate failure to the host PC at this point, so we'll just
                        // pretend that the entire policy delay has elapsed.  This will trigger an immediate wake when the power source becomes active again.
                        error!(
                                "[Time/Alarm] Failed to get current datetime when transitioning expired timer waiting for policy delay to inactive state"
                            );
                        u32::MAX
                    }
                };

                timer_state.wake_state = WakeState::ExpiredWaitingForPowerSource(total_seconds_elapsed_on_policy_delay);
                self.timer_signal.signal(None);
            }
        });
    }

    pub(crate) async fn wait_until_wake(&self, clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>) {
        loop {
            let mut wait_duration: Option<u32> = self.timer_signal.wait().await;
            'waiting_for_timer: loop {
                match wait_duration {
                    Some(seconds) => {
                        match select(
                            embassy_time::Timer::after_secs(seconds.into()),
                            self.timer_signal.wait(),
                        )
                        .await
                        {
                            Either::First(()) => {
                                if self.process_expired_timer(clock_state) {
                                    return;
                                }
                            }
                            Either::Second(new_wait_duration) => {
                                wait_duration = new_wait_duration;
                            }
                        }
                    }
                    None => {
                        // Wait until a new timer command comes in
                        break 'waiting_for_timer;
                    }
                }
            }
        }
    }

    /// Handles state changes for when the timer expires (figuring out what to do based on the current power source, etc).
    /// Returns true if the timer's expiry indicates that a wake event should be signaled to the host.
    fn process_expired_timer(&self, clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>) -> bool {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            match timer_state.wake_state {
                // Clear: timer was disarmed right as we were waking - nothing to do.
                // ExpiredOrphaned: shouldn't happen, but if we're in this state the timer should be dead, so nothing to do.
                // ExpiredWaitingForPowerSource: shouldn't happen, but if we're in this state the timer is still waiting for power source so nothing to do.
                WakeState::Clear | WakeState::ExpiredOrphaned | WakeState::ExpiredWaitingForPowerSource(_) => {
                    return false;
                }

                WakeState::Armed | WakeState::ExpiredWaitingForPolicyDelay(_, _) => {
                    let expiration_time = match timer_state.persistent_storage.get_expiration_time() {
                        Some(expiration_time) => expiration_time,
                        None => {
                            error!(
                                "[Time/Alarm] Timer expired when no expiration time was set - this should never happen"
                            );
                            return false;
                        }
                    };

                    match Self::get_current_datetime(clock_state) {
                        Ok(now) => {
                            if now.to_unix_time_seconds() < expiration_time.to_unix_time_seconds() {
                                // Time hasn't actually passed the mark yet - this can happen if we were reprogrammed with a different time right as the old timer was expiring. Reset the timer.
                                timer_state.wake_state = WakeState::Armed;
                                self.timer_signal.signal(Some(
                                    expiration_time
                                        .to_unix_time_seconds()
                                        .saturating_sub(now.to_unix_time_seconds())
                                        as u32,
                                ));
                                return false;
                            }
                        }
                        Err(_) => {
                            // This should never happen, because it means the clock is not working after we've successfully initialized (which
                            // requires the clock to be working).
                            // If it does, though, we don't have a way to communicate failure to the host PC at this point, so we'll just
                            // wake the device immediately on the assumption that the alarm has actually expired.  This gets it wrong in the case
                            // where the timer is reprogrammed immediately as it expires, but that's an extremely rare case and we can't do better
                            // than that if our clock is broken.
                            error!("[Time/Alarm] Failed to get current datetime when processing expired timer");
                        }
                    }

                    timer_state.timer_status.set_timer_expired(true);
                    if timer_state.is_active {
                        timer_state.timer_status.set_timer_triggered_wake(true);
                        timer_state
                            .persistent_storage
                            .set_timer_wake_policy(AlarmExpiredWakePolicy::NEVER);
                        self.clear_expiration_time(&mut timer_state);
                        return true;
                    } else {
                        if timer_state.persistent_storage.get_timer_wake_policy() == AlarmExpiredWakePolicy::NEVER {
                            timer_state.wake_state = WakeState::ExpiredOrphaned;
                            return false;
                        }

                        if let WakeState::ExpiredWaitingForPolicyDelay(_, _) = timer_state.wake_state {
                            timer_state.wake_state = WakeState::ExpiredWaitingForPowerSource(
                                timer_state.persistent_storage.get_timer_wake_policy().0,
                            );
                        } else {
                            timer_state.wake_state = WakeState::ExpiredWaitingForPowerSource(0);
                        }
                    }
                }
            }

            false
        })
    }

    fn clear_expiration_time(&self, timer_state: &mut TimerState) {
        timer_state.persistent_storage.set_expiration_time(None);
        timer_state.wake_state = WakeState::Clear;
        self.timer_signal.signal(None);
    }

    fn get_current_datetime(
        clock_state: &Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>,
    ) -> Result<Datetime, DatetimeClockError> {
        clock_state.lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime())
    }
}
