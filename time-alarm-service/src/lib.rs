#![no_std]

use core::cell::RefCell;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClock, DatetimeClockError};
use embedded_services::{GlobalRawMutex, comms::MailboxDelegateError};
use embedded_services::{comms, info, warn};
use time_alarm_service_messages::*;

pub mod task;
mod timer;
use timer::Timer;

// -------------------------------------------------

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TimeAlarmError {
    UnknownCommand,
    DoubleInitError,
    MailboxFullError,
    ClockError(DatetimeClockError),
}

impl From<TimeAlarmError> for MailboxDelegateError {
    fn from(error: TimeAlarmError) -> Self {
        match error {
            TimeAlarmError::UnknownCommand => MailboxDelegateError::InvalidData,
            TimeAlarmError::DoubleInitError => MailboxDelegateError::Other,
            TimeAlarmError::MailboxFullError => MailboxDelegateError::BufferFull,
            TimeAlarmError::ClockError(_) => MailboxDelegateError::Other,
        }
    }
}

impl From<DatetimeClockError> for TimeAlarmError {
    fn from(e: DatetimeClockError) -> Self {
        TimeAlarmError::ClockError(e)
    }
}

impl From<embedded_services::intrusive_list::Error> for TimeAlarmError {
    fn from(_error: embedded_services::intrusive_list::Error) -> Self {
        TimeAlarmError::DoubleInitError
    }
}

// -------------------------------------------------

mod time_zone_data {
    use crate::AcpiDaylightSavingsTimeStatus;
    use crate::AcpiTimeZone;
    use crate::NvramStorage;

    pub struct TimeZoneData {
        // Storage used to back the timezone and DST settings.
        storage: &'static mut dyn NvramStorage<'static, u32>,
    }

    #[repr(C)]
    #[derive(zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable, Copy, Clone, Debug)]
    struct RawTimeZoneData {
        tz: i16,
        dst: u8,
        _padding: u8,
    }

    impl TimeZoneData {
        pub fn new(storage: &'static mut dyn NvramStorage<'static, u32>) -> Self {
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
            (|| -> Result<(AcpiTimeZone, AcpiDaylightSavingsTimeStatus), time_alarm_service_messages::AcpiTimeAlarmError> {
                Ok((representation.tz.try_into()?, representation.dst.try_into()?))
            })()
            .unwrap_or((AcpiTimeZone::Unknown, AcpiDaylightSavingsTimeStatus::NotObserved))
        }
    }
}
use time_zone_data::TimeZoneData;

// -------------------------------------------------

struct ClockState {
    datetime_clock: &'static mut dyn DatetimeClock,
    tz_data: TimeZoneData,
}

// -------------------------------------------------

struct Timers {
    ac_timer: Timer,
    dc_timer: Timer,
}

impl Timers {
    fn get_timer(&self, timer: AcpiTimerId) -> &Timer {
        match timer {
            AcpiTimerId::AcPower => &self.ac_timer,
            AcpiTimerId::DcPower => &self.dc_timer,
        }
    }

    fn new(
        ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Self {
        Self {
            ac_timer: Timer::new(ac_expiration_storage, ac_policy_storage),
            dc_timer: Timer::new(dc_expiration_storage, dc_policy_storage),
        }
    }
}

// -------------------------------------------------

pub struct Service {
    endpoint: comms::Endpoint,

    // ACPI messages from the host are sent through this channel.
    acpi_channel: Channel<GlobalRawMutex, (comms::EndpointID, AcpiTimeAlarmRequest), 10>,

    clock_state: Mutex<GlobalRawMutex, RefCell<ClockState>>,

    // TODO [POWER_SOURCE] signal this whenever the power source changes
    power_source_signal: Signal<GlobalRawMutex, AcpiTimerId>,

    timers: Timers,

    capabilities: TimeAlarmDeviceCapabilities,
}

impl Service {
    pub async fn init(
        service_storage: &'static OnceLock<Service>,
        backing_clock: &'static mut impl DatetimeClock,
        tz_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Result<&'static Service, TimeAlarmError> {
        info!("Starting time-alarm service task");

        let service = service_storage.get_or_init(|| Service {
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::TimeAlarm)),
            acpi_channel: Channel::new(),
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

        comms::register_endpoint(service, &service.endpoint).await?;

        Ok(service)
    }

    pub(crate) async fn handle_requests(&'static self) -> ! {
        loop {
            let acpi_command = self.acpi_channel.receive();
            let power_source_change = self.power_source_signal.wait();

            match select(acpi_command, power_source_change).await {
                Either::First((respond_to_endpoint, acpi_command)) => {
                    info!("[Time/Alarm] Received command: {:?}", acpi_command);

                    let result: AcpiTimeAlarmResult = self
                        .handle_acpi_command(acpi_command)
                        .await
                        .map_err(|_| time_alarm_service_messages::AcpiTimeAlarmError::UnspecifiedFailure);

                    info!("[Time/Alarm] Responding with: {:?}", result);

                    let _: Result<(), core::convert::Infallible> =
                        self.endpoint.send(respond_to_endpoint, &result).await;
                }
                Either::Second(new_power_source) => {
                    info!("[Time/Alarm] Power source changed to {:?}", new_power_source);

                    self.timers
                        .get_timer(new_power_source.get_other_timer_id())
                        .set_active(&self.clock_state, false);
                    self.timers
                        .get_timer(new_power_source)
                        .set_active(&self.clock_state, true);
                }
            }
        }
    }

    pub(crate) async fn handle_timer(&'static self, timer_id: AcpiTimerId) -> ! {
        let timer = self.timers.get_timer(timer_id);
        loop {
            timer.wait_until_wake(&self.clock_state).await;
            let _ = self
                .timers
                .get_timer(timer_id.get_other_timer_id())
                .set_timer_wake_policy(&self.clock_state, AlarmExpiredWakePolicy::NEVER);

            warn!(
                "[Time/Alarm] Timer {:?} expired and would trigger a wake now, but the power service is not yet implemented so will currently do nothing",
                timer_id
            );
            // TODO [COMMS] We can't currently trigger a wake because the power service isn't implemented yet - when it is, we need to notify it here
        }
    }

    async fn handle_acpi_command(
        &'static self,
        command: AcpiTimeAlarmRequest,
    ) -> Result<AcpiTimeAlarmResponse, TimeAlarmError> {
        match command {
            AcpiTimeAlarmRequest::GetCapabilities => Ok(AcpiTimeAlarmResponse::Capabilities(self.capabilities)),
            AcpiTimeAlarmRequest::GetRealTime => self.clock_state.lock(|clock_state| {
                let clock_state = clock_state.borrow();
                let datetime = clock_state.datetime_clock.get_current_datetime()?;
                let (time_zone, dst_status) = clock_state.tz_data.get_data();
                Ok(AcpiTimeAlarmResponse::RealTime(AcpiTimestamp {
                    datetime,
                    time_zone,
                    dst_status,
                }))
            }),
            AcpiTimeAlarmRequest::SetRealTime(timestamp) => self.clock_state.lock(|clock_state| {
                let mut clock_state = clock_state.borrow_mut();
                clock_state.datetime_clock.set_current_datetime(&timestamp.datetime)?;
                clock_state.tz_data.set_data(timestamp.time_zone, timestamp.dst_status);

                Ok(AcpiTimeAlarmResponse::OkNoData)
            }),
            AcpiTimeAlarmRequest::GetWakeStatus(timer_id) => {
                let status = self.timers.get_timer(timer_id).get_wake_status();
                Ok(AcpiTimeAlarmResponse::TimerStatus(status))
            }
            AcpiTimeAlarmRequest::ClearWakeStatus(timer_id) => {
                self.timers.get_timer(timer_id).clear_wake_status();
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::SetExpiredTimerPolicy(timer_id, timer_policy) => {
                self.timers
                    .get_timer(timer_id)
                    .set_timer_wake_policy(&self.clock_state, timer_policy)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::SetTimerValue(timer_id, timer_value) => {
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
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetExpiredTimerPolicy(timer_id) => Ok(AcpiTimeAlarmResponse::WakePolicy(
                self.timers.get_timer(timer_id).get_timer_wake_policy(),
            )),
            AcpiTimeAlarmRequest::GetTimerValue(timer_id) => {
                let expiration_time = self.timers.get_timer(timer_id).get_expiration_time();

                let timer_wire_format = match expiration_time {
                    Some(expiration_time) => {
                        let current_time = self
                            .clock_state
                            .lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime())?;

                        AlarmTimerSeconds(
                            expiration_time
                                .to_unix_time_seconds()
                                .saturating_sub(current_time.to_unix_time_seconds()) as u32,
                        )
                    }
                    None => AlarmTimerSeconds::DISABLED,
                };

                Ok(AcpiTimeAlarmResponse::TimerSeconds(timer_wire_format))
            }
        }
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(acpi_cmd) = message.data.get::<time_alarm_service_messages::AcpiTimeAlarmRequest>() {
            self.acpi_channel
                .try_send((message.from, *acpi_cmd))
                .map_err(|_| MailboxDelegateError::BufferFull)?;
            Ok(())
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}
