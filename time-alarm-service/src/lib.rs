#![no_std]

use core::cell::RefCell;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClock, DatetimeClockError};
use embedded_services::GlobalRawMutex;
use embedded_services::{info, warn};
use time_alarm_service_messages::*;

pub mod task;
mod timer;
use timer::Timer;

// -------------------------------------------------

mod time_zone_data {
    use crate::AcpiDaylightSavingsTimeStatus;
    use crate::AcpiTimeZone;
    use crate::NvramStorage;

    pub struct TimeZoneData<'hw> {
        // Storage used to back the timezone and DST settings.
        storage: &'hw mut dyn NvramStorage<'hw, u32>,
    }

    #[repr(C)]
    #[derive(zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable, Copy, Clone, Debug)]
    struct RawTimeZoneData {
        tz: i16,
        dst: u8,
        _padding: u8,
    }

    impl<'hw> TimeZoneData<'hw> {
        pub fn new(storage: &'hw mut dyn NvramStorage<'hw, u32>) -> Self {
            Self { storage }
        }

        /// Writes the given time zone and daylight savings time status to NVRAM.
        ///
        pub fn set_data(&mut self, tz: AcpiTimeZone, dst: AcpiDaylightSavingsTimeStatus) {
            let representation = RawTimeZoneData {
                tz: tz.into(),
                dst: dst.into(),
                _padding: 0,
            };

            self.storage.write(zerocopy::transmute!(representation));
        }

        /// Retrieves the current time zone / daylight savings time.
        /// If the stored data is invalid, implying that the NVRAM has never been initialized, defaults to
        /// (AcpiTimeZone::Unknown, AcpiDaylightSavingsTimeStatus::NotObserved).
        ///
        pub fn get_data(&self) -> (AcpiTimeZone, AcpiDaylightSavingsTimeStatus) {
            let representation: RawTimeZoneData = zerocopy::transmute!(self.storage.read());

            let time_zone = AcpiTimeZone::try_from(representation.tz).unwrap_or(AcpiTimeZone::Unknown);
            let dst_status = AcpiDaylightSavingsTimeStatus::try_from(representation.dst)
                .unwrap_or(AcpiDaylightSavingsTimeStatus::NotObserved);
            (time_zone, dst_status)
        }
    }
}
use time_zone_data::TimeZoneData;

// -------------------------------------------------

struct ClockState<'hw> {
    datetime_clock: &'hw mut dyn DatetimeClock,
    tz_data: TimeZoneData<'hw>,
}

// -------------------------------------------------

struct Timers<'hw> {
    ac_timer: Timer<'hw>,
    dc_timer: Timer<'hw>,
}

impl<'hw> Timers<'hw> {
    fn get_timer(&self, timer: AcpiTimerId) -> &Timer<'hw> {
        match timer {
            AcpiTimerId::AcPower => &self.ac_timer,
            AcpiTimerId::DcPower => &self.dc_timer,
        }
    }

    fn new(
        ac_expiration_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        ac_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        dc_expiration_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        dc_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
    ) -> Self {
        Self {
            ac_timer: Timer::new(ac_expiration_storage, ac_policy_storage),
            dc_timer: Timer::new(dc_expiration_storage, dc_policy_storage),
        }
    }
}

// -------------------------------------------------

pub struct Service<'hw> {
    clock_state: Mutex<GlobalRawMutex, RefCell<ClockState<'hw>>>,

    // TODO [POWER_SOURCE] signal this whenever the power source changes
    power_source_signal: Signal<GlobalRawMutex, AcpiTimerId>,

    timers: Timers<'hw>,

    capabilities: TimeAlarmDeviceCapabilities,
}

impl<'hw> Service<'hw> {
    pub async fn init(
        service_storage: &'hw OnceLock<Service<'hw>>,
        backing_clock: &'hw mut impl DatetimeClock,
        tz_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        ac_expiration_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        ac_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        dc_expiration_storage: &'hw mut dyn NvramStorage<'hw, u32>,
        dc_policy_storage: &'hw mut dyn NvramStorage<'hw, u32>,
    ) -> Result<&'hw Service<'hw>, DatetimeClockError> {
        info!("Starting time-alarm service task");

        let service = service_storage.get_or_init(|| Service {
            clock_state: Mutex::new(RefCell::new(ClockState {
                datetime_clock: backing_clock,
                tz_data: TimeZoneData::new(tz_storage),
            })),
            power_source_signal: Signal::new(),
            timers: Timers::new(
                ac_expiration_storage,
                ac_policy_storage,
                dc_expiration_storage,
                dc_policy_storage,
            ),
            capabilities: {
                // TODO [CONFIG] We could consider making some of these user-configurable, e.g. if we want to support devices that don't have a battery
                let mut caps = TimeAlarmDeviceCapabilities(0);
                caps.set_ac_wake_implemented(true);
                caps.set_dc_wake_implemented(true);
                caps.set_realtime_implemented(true);
                caps.set_realtime_accuracy_in_milliseconds(false);
                caps.set_get_wake_status_supported(true);
                caps.set_ac_s4_wake_supported(true);
                caps.set_ac_s5_wake_supported(true);
                caps.set_dc_s4_wake_supported(true);
                caps.set_dc_s5_wake_supported(true);
                caps
            },
        });

        // TODO [POWER_SOURCE] we need to subscribe to messages that tell us if we're on AC or DC power so we can decide which alarms to trigger, but those notifications are not yet implemented - revisit when they are.
        // TODO [POWER_SOURCE] if it's possible to learn which power source is active at init time, we should set that one active rather than defaulting to the AC timer.
        service.timers.ac_timer.start(&service.clock_state, true)?;
        service.timers.dc_timer.start(&service.clock_state, false)?;

        Ok(service)
    }

    /// Query clock capabilities.  Analogous to ACPI TAD's _GRT method.
    pub fn get_capabilities(&self) -> TimeAlarmDeviceCapabilities {
        self.capabilities
    }

    /// Query the current time.  Analogous to ACPI TAD's _GRT method.
    pub fn get_real_time(&self) -> Result<AcpiTimestamp, DatetimeClockError> {
        self.clock_state.lock(|clock_state| {
            let clock_state = clock_state.borrow();
            let datetime = clock_state.datetime_clock.get_current_datetime()?;
            let (time_zone, dst_status) = clock_state.tz_data.get_data();
            Ok(AcpiTimestamp {
                datetime,
                time_zone,
                dst_status,
            })
        })
    }

    /// Change the current time.  Analogous to ACPI TAD's _SRT method.
    pub fn set_real_time(&self, timestamp: AcpiTimestamp) -> Result<(), DatetimeClockError> {
        self.clock_state.lock(|clock_state| {
            let mut clock_state = clock_state.borrow_mut();
            clock_state.datetime_clock.set_current_datetime(&timestamp.datetime)?;
            clock_state.tz_data.set_data(timestamp.time_zone, timestamp.dst_status);
            Ok(())
        })
    }

    /// Query the current wake status.  Analogous to ACPI TAD's _GWS method.
    pub fn get_wake_status(&self, timer_id: AcpiTimerId) -> TimerStatus {
        self.timers.get_timer(timer_id).get_wake_status()
    }

    /// Clear the current wake status.  Analogous to ACPI TAD's _CWS method.
    pub fn clear_wake_status(&self, timer_id: AcpiTimerId) {
        self.timers.get_timer(timer_id).clear_wake_status();
    }

    /// Configures behavior when the timer expires while the system is on the other power source.  Analogous to ACPI TAD's _STP method.
    pub fn set_expired_timer_policy(
        &self,
        timer_id: AcpiTimerId,
        policy: AlarmExpiredWakePolicy,
    ) -> Result<(), DatetimeClockError> {
        self.timers
            .get_timer(timer_id)
            .set_timer_wake_policy(&self.clock_state, policy)?;
        Ok(())
    }

    /// Query current behavior when the timer expires while the system is on the other power source.  Analogous to ACPI TAD's _TIP method.
    pub fn get_expired_timer_policy(&self, timer_id: AcpiTimerId) -> AlarmExpiredWakePolicy {
        self.timers.get_timer(timer_id).get_timer_wake_policy()
    }

    /// Change the expiry time for the given timer.  Analogous to ACPI TAD's _STV method.
    pub fn set_timer_value(
        &self,
        timer_id: AcpiTimerId,
        timer_value: AlarmTimerSeconds,
    ) -> Result<(), DatetimeClockError> {
        let new_expiration_time = match timer_value {
            AlarmTimerSeconds::DISABLED => None,
            AlarmTimerSeconds(secs) => {
                let current_time = self
                    .clock_state
                    .lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime())?;

                Some(Datetime::from_unix_time_seconds(
                    current_time.to_unix_time_seconds() + u64::from(secs),
                ))
            }
        };

        self.timers
            .get_timer(timer_id)
            .set_expiration_time(&self.clock_state, new_expiration_time)?;
        Ok(())
    }

    /// Query the expiry time for the given timer.  Analogous to ACPI TAD's _TIV method.
    pub fn get_timer_value(&self, timer_id: AcpiTimerId) -> Result<AlarmTimerSeconds, DatetimeClockError> {
        let expiration_time = self.timers.get_timer(timer_id).get_expiration_time();
        match expiration_time {
            Some(expiration_time) => {
                let current_time = self
                    .clock_state
                    .lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime())?;

                Ok(AlarmTimerSeconds(
                    expiration_time
                        .to_unix_time_seconds()
                        .saturating_sub(current_time.to_unix_time_seconds()) as u32,
                ))
            }
            None => Ok(AlarmTimerSeconds::DISABLED),
        }
    }

    pub(crate) async fn run_service(&'hw self) -> ! {
        loop {
            embassy_futures::select::select3(
                self.handle_power_source_updates(),
                self.handle_timer(AcpiTimerId::AcPower),
                self.handle_timer(AcpiTimerId::DcPower),
            )
            .await;
        }
    }

    async fn handle_power_source_updates(&'hw self) -> ! {
        loop {
            let new_power_source = self.power_source_signal.wait().await;
            info!("[Time/Alarm] Power source changed to {:?}", new_power_source);

            self.timers
                .get_timer(new_power_source.get_other_timer_id())
                .set_active(&self.clock_state, false);
            self.timers
                .get_timer(new_power_source)
                .set_active(&self.clock_state, true);
        }
    }

    async fn handle_timer(&'hw self, timer_id: AcpiTimerId) -> ! {
        let timer = self.timers.get_timer(timer_id);
        loop {
            timer.wait_until_wake(&self.clock_state).await;
            self.timers
                .get_timer(timer_id.get_other_timer_id())
                .set_timer_wake_policy(&self.clock_state, AlarmExpiredWakePolicy::NEVER)
                .unwrap_or_else(|e| {
                    warn!(
                        "[Time/Alarm] Failed to update wake policy on timer expiry - this should never happen: {:?}",
                        e
                    );
                });

            warn!(
                "[Time/Alarm] Timer {:?} expired and would trigger a wake now, but the power service is not yet implemented so will currently do nothing",
                timer_id
            );
            // TODO [COMMS] We can't currently trigger a wake because the power service isn't implemented yet - when it is, we need to notify it here
        }
    }
}

impl<'hw> embedded_services::relay::mctp::RelayServiceHandlerTypes for Service<'hw> {
    type RequestType = AcpiTimeAlarmRequest;
    type ResultType = AcpiTimeAlarmResult;
}

impl<'hw> embedded_services::relay::mctp::RelayServiceHandler for Service<'hw> {
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        use time_alarm_service_messages::AcpiTimeAlarmResponse;
        match request {
            AcpiTimeAlarmRequest::GetCapabilities => Ok(AcpiTimeAlarmResponse::Capabilities(self.get_capabilities())),
            AcpiTimeAlarmRequest::GetRealTime => Ok(AcpiTimeAlarmResponse::RealTime(self.get_real_time()?)),
            AcpiTimeAlarmRequest::SetRealTime(timestamp) => {
                self.set_real_time(timestamp)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetWakeStatus(timer_id) => {
                Ok(AcpiTimeAlarmResponse::TimerStatus(self.get_wake_status(timer_id)))
            }
            AcpiTimeAlarmRequest::ClearWakeStatus(timer_id) => {
                self.clear_wake_status(timer_id);
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::SetExpiredTimerPolicy(timer_id, timer_policy) => {
                self.set_expired_timer_policy(timer_id, timer_policy)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetExpiredTimerPolicy(timer_id) => Ok(AcpiTimeAlarmResponse::WakePolicy(
                self.get_expired_timer_policy(timer_id),
            )),
            AcpiTimeAlarmRequest::SetTimerValue(timer_id, timer_value) => {
                self.set_timer_value(timer_id, timer_value)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetTimerValue(timer_id) => {
                Ok(AcpiTimeAlarmResponse::TimerSeconds(self.get_timer_value(timer_id)?))
            }
        }
    }
}
