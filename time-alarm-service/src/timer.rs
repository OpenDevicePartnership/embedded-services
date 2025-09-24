use crate::{AlarmExpiredWakePolicy, TimerStatus};
use core::cell::RefCell;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::Mutex, signal::Signal};
use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::Datetime;
use embedded_services::GlobalRawMutex;

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
    // Expired while the policy was set to NEVER, so the timer is effectively dead until reprogrammed
    ExpiredOrphaned,
}

// TODO do we want to do something like this to make persistent storage easier to read?
// struct PersistentAlarmExpiredWakePolicy {
//     wake_policy: AlarmExpiredWakePolicy,
//     storage: &'static mut dyn NvramStorage<'static, u32>,
// }
// struct PersistentExpirationTime {
//     expiration_time: Option<Datetime>,
//     storage: &'static mut dyn NvramStorage<'static, u32>,
// }

struct TimerState {
    /// When the timer is programmed to expire, or None if the timer is not set
    /// This can't be part of the wake_state because we need to be able to report its value for _CWS even when the timer has expired and
    /// we're handling the power source policy.
    expiration_time_storage: &'static mut dyn NvramStorage<'static, u32>,

    // Persistent storage for the AlarmExpiredWakePolicy
    wake_policy_storage: &'static mut dyn NvramStorage<'static, u32>,

    wake_state: WakeState,

    timer_status: TimerStatus,

    // Whether or not this timer is currently active (i.e. the system is on the power source this timer manages)
    // Even if it's not active, it still counts down if it's programmed - it just won't trigger a wake event if it expires while inactive.
    is_active: bool,
}

impl TimerState {
    const NO_EXPIRATION_TIME: u32 = u32::MAX;

    fn get_timer_wake_policy(&self) -> AlarmExpiredWakePolicy {
        AlarmExpiredWakePolicy(self.wake_policy_storage.read())
    }

    fn set_timer_wake_policy(&mut self, wake_policy: AlarmExpiredWakePolicy) {
        self.wake_policy_storage.write(wake_policy.0);
    }

    fn get_expiration_time(&self) -> Option<Datetime> {
        match self.expiration_time_storage.read() {
            Self::NO_EXPIRATION_TIME => None,
            secs => Some(Datetime::from_unix_time_seconds(secs.into())),
        }
    }

    fn set_expiration_time(&mut self, expiration_time: Option<Datetime>) {
        match expiration_time {
            Some(dt) => {
                self.expiration_time_storage
                    .write(dt.to_unix_time_seconds().try_into().expect(
                        "Datetime::to_unix_timestamp() returns i64, which should always fit in u32 until the year 2106",
                    ));
                self.wake_state = WakeState::Armed;
            }
            None => {
                self.expiration_time_storage.write(Self::NO_EXPIRATION_TIME);
                self.wake_state = WakeState::Clear;
            }
        }
    }
}

pub(crate) struct Timer {
    timer_state: Mutex<GlobalRawMutex, RefCell<TimerState>>,

    timer_signal: Signal<GlobalRawMutex, Option<u32>>,
}

impl Timer {
    pub fn new(
        active: bool,
        expiration_time_storage: &'static mut dyn NvramStorage<'static, u32>,
        wake_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Self {
        let result = Self {
            timer_state: Mutex::new(RefCell::new(TimerState {
                expiration_time_storage,
                wake_policy_storage,

                wake_state: WakeState::Clear,

                timer_status: Default::default(),
                is_active: false,
            })),
            timer_signal: Signal::new(),
        };

        // TODO make sure there's not some weird edge case here with coming back from a power loss - we may need to suppress the wake policy timer in this case?
        result.set_timer_wake_policy(
            result
                .timer_state
                .lock(|timer_state| timer_state.borrow().get_timer_wake_policy()),
        );

        result.set_expiration_time(
            result
                .timer_state
                .lock(|timer_state| timer_state.borrow().get_expiration_time()),
        );

        result.set_active(active);

        result
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

    // TODO [SPEC_QUESTION] the spec is ambiguous on whether or not this policy should include the number of seconds that have elapsed against it
    //     (i.e. if the user set it to 60s and 45s have elapsed since we switched to the expired power source, should we report
    //      60s or 15s?)- see if we can get a concrete answer on this.
    //
    pub fn get_timer_wake_policy(&self) -> AlarmExpiredWakePolicy {
        self.timer_state
            .lock(|timer_state| timer_state.borrow().get_timer_wake_policy())
    }

    pub fn set_timer_wake_policy(&self, wake_policy: AlarmExpiredWakePolicy) {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();
            timer_state.set_timer_wake_policy(wake_policy);

            // TODO [SPEC_QUESTION] verify this is correct - the spec isn't particularly clear on what should happen if reprogramming the policy while it's actively ticking down,
            //      may need to look at the windows acpi implementation or something
            //
            if let WakeState::ExpiredWaitingForPolicyDelay(_, _) = timer_state.wake_state {
                timer_state.wake_state = WakeState::ExpiredWaitingForPolicyDelay(
                    crate::get_current_datetime()
                        .expect("Datetime clock should have already been initialized before we were constructed"),
                    0,
                );
                self.timer_signal.signal(Some(wake_policy.0));
            }
        })
    }

    pub fn set_expiration_time(&self, expiration_time: Option<Datetime>) {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            // Per ACPI 6.4 section 9.18.1: "The status of wake timers can be reset by setting the wake alarm".
            timer_state.timer_status = Default::default();

            match expiration_time {
                Some(dt) => {
                    timer_state.set_expiration_time(expiration_time);
                    timer_state.wake_state = WakeState::Armed;

                    // Note: If the expiration time was in the past, this will immediately trigger the timer to expire.
                    self.timer_signal.signal(Some(
                        dt
                            .to_unix_time_seconds()
                            .saturating_sub(crate::get_current_datetime().expect("datetime clocks should have been initialized before we were constructed").to_unix_time_seconds()).try_into()
                            .expect("Users should not have been able to program a time greater than u32::MAX seconds in the future - the ACPI spec prevents it")
                    ));
                }
                None => self.clear_expiration_time(&mut timer_state)
            }
        });
    }

    pub fn get_expiration_time(&self) -> Option<Datetime> {
        self.timer_state
            .lock(|timer_state| timer_state.borrow().get_expiration_time())
    }

    pub fn set_active(&self, is_active: bool) {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            let was_active = timer_state.is_active;
            timer_state.is_active = is_active;

            if was_active == is_active {
                return; // No change
            }

            if !was_active {
                if let WakeState::ExpiredWaitingForPowerSource(seconds_already_elapsed) = timer_state.wake_state {
                    timer_state.wake_state = WakeState::ExpiredWaitingForPolicyDelay(
                        crate::get_current_datetime()
                            .expect("datetime clock should have already been initialized before we were constructed"),
                        seconds_already_elapsed,
                    );
                    self.timer_signal.signal(Some(
                        timer_state
                            .get_timer_wake_policy()
                            .0
                            .saturating_sub(seconds_already_elapsed),
                    ));
                }
            } else if was_active {
                if let WakeState::ExpiredWaitingForPolicyDelay(wait_start_time, seconds_elapsed_before_wait) =
                    timer_state.wake_state
                {
                    let total_seconds_elapsed_on_policy_delay: u32 = seconds_elapsed_before_wait
                        + (crate::get_current_datetime()
                            .expect("Datetime clock should have already been initialized before we were constructed")
                            .to_unix_time_seconds()
                            .saturating_sub(wait_start_time.to_unix_time_seconds())) as u32; // TODO figure out how to make this build without the cast

                    timer_state.wake_state =
                        WakeState::ExpiredWaitingForPowerSource(total_seconds_elapsed_on_policy_delay);
                    self.timer_signal.signal(None);
                }
            }
        });
    }

    pub(crate) async fn wait_until_wake(&self) {
        let mut wait_duration: Option<u32> = self.timer_signal.wait().await;

        loop {
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
                                if self.process_expired_timer() {
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
    fn process_expired_timer(&self) -> bool {
        self.timer_state.lock(|timer_state| {
            let mut timer_state = timer_state.borrow_mut();

            match timer_state.wake_state {
                // Clear: timer was disarmed right as we were waking - nothing to do.
                // ExpiredOrphaned: shouldn't happen, but if we're in this state the timer should be dead, so nothing to do.
                // ExpiredWaitingForPowerSource: shouldn't happen, but if we're in this state the timer is still waiting for power source so nothing to do.
                WakeState::Clear | WakeState::ExpiredOrphaned | WakeState::ExpiredWaitingForPowerSource(_) => return false,

                WakeState::Armed | WakeState::ExpiredWaitingForPolicyDelay(_, _) => {
                    let now = crate::get_current_datetime().expect("Datetime clock should have already been initialized before we were constructed");
                    let expiration_time = timer_state.get_expiration_time().expect("We should never be in the Armed or ExpiredWaitingForPolicyDelay states if there's no expiration time set");
                    if now.to_unix_time_seconds() < expiration_time.to_unix_time_seconds() { // TODO we should probably implement Ord for Datetime and use that
                        // Time hasn't actually passed the mark yet - this can happen if we were reprogrammed with a different time right as the old timer was expiring. Reset the timer.
                        timer_state.wake_state = WakeState::Armed;
                        self.timer_signal.signal(Some(expiration_time
                            .to_unix_time_seconds()
                            .saturating_sub(now.to_unix_time_seconds())
                            .try_into()
                            .expect("Users should not have been able to program a time greater than u32::MAX seconds in the future - the ACPI spec prevents it")));
                        return false;
                    }

                    timer_state.timer_status.timer_expired = true;
                    if timer_state.is_active {
                        timer_state.timer_status.timer_triggered_wake = true;
                        self.clear_expiration_time(&mut timer_state);
                        return true;
                    }
                    else {
                        if timer_state.get_timer_wake_policy() == AlarmExpiredWakePolicy::NEVER {
                            timer_state.wake_state = WakeState::ExpiredOrphaned;
                            return false;
                        }

                        if let WakeState::ExpiredWaitingForPolicyDelay(_, _) = timer_state.wake_state {
                            timer_state.wake_state = WakeState::ExpiredWaitingForPowerSource(timer_state.get_timer_wake_policy().0);
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
        timer_state.set_expiration_time(None);
        timer_state.set_timer_wake_policy(AlarmExpiredWakePolicy::NEVER);
        timer_state.wake_state = WakeState::Clear;
        self.timer_signal.signal(None);
    }
}
