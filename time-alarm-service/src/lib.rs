#![no_std]

use core::any::Any;
use core::array::TryFromSliceError;
use core::borrow::Borrow;
use core::cell::RefCell;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embedded_services::ec_type::message::AcpiMsgComms;
use embedded_services::{GlobalRawMutex, comms::MailboxDelegateError};

use embedded_mcu_hal::NvramStorage;
use embedded_mcu_hal::time::{Datetime, DatetimeClock, DatetimeClockError};
use embedded_services::{comms, error, info, trace};

mod acpi_timestamp;
use acpi_timestamp::{AcpiDaylightSavingsTimeStatus, AcpiTimeZone, AcpiTimestamp};

mod timer;
use timer::Timer;

// -------------------------------------------------

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TimeAlarmError {
    UnknownCommand,
    DoubleInitError,
    MailboxFullError,
    InvalidAcpiTimerId,
    InvalidArgument,
    ClockError(DatetimeClockError),
}

impl From<TimeAlarmError> for MailboxDelegateError {
    fn from(error: TimeAlarmError) -> Self {
        match error {
            TimeAlarmError::UnknownCommand => MailboxDelegateError::InvalidData,
            TimeAlarmError::DoubleInitError => {
                panic!("Should never attempt intitialization as a response to receiving a mailbox message")
            }
            TimeAlarmError::MailboxFullError => MailboxDelegateError::BufferFull,
            TimeAlarmError::InvalidAcpiTimerId => MailboxDelegateError::InvalidData,
            TimeAlarmError::InvalidArgument => MailboxDelegateError::InvalidData,
            TimeAlarmError::ClockError(_) => MailboxDelegateError::Other,
        }
    }
}

impl From<TryFromSliceError> for TimeAlarmError {
    fn from(_error: TryFromSliceError) -> Self {
        TimeAlarmError::InvalidArgument
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

impl From<embedded_mcu_hal::time::DatetimeError> for TimeAlarmError {
    fn from(_error: embedded_mcu_hal::time::DatetimeError) -> Self {
        TimeAlarmError::InvalidArgument
    }
}

// -------------------------------------------------

// Timer ID as defined in the ACPI spec.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AcpiTimerId {
    AcPower,
    DcPower,
}

impl AcpiTimerId {
    // Given a byte slice, attempts to parse an AcpiTimerId from the first 4 bytes.
    // Returns the parsed AcpiTimerId and a slice of the remaining bytes.
    fn try_from_bytes(bytes: &'_ [u8]) -> Result<(Self, &'_ [u8]), TimeAlarmError> {
        const SIZE_BYTES: usize = core::mem::size_of::<u32>();
        let id = u32::from_le_bytes(
            bytes
                .get(0..SIZE_BYTES)
                .ok_or(TimeAlarmError::InvalidArgument)?
                .try_into()?,
        );

        Ok((AcpiTimerId::try_from(id)?, &bytes[SIZE_BYTES..]))
    }

    fn get_other_timer_id(&self) -> Self {
        match self {
            AcpiTimerId::AcPower => AcpiTimerId::DcPower,
            AcpiTimerId::DcPower => AcpiTimerId::AcPower,
        }
    }
}

impl TryFrom<u32> for AcpiTimerId {
    type Error = TimeAlarmError;

    fn try_from(value: u32) -> Result<Self, TimeAlarmError> {
        match value {
            0 => Ok(AcpiTimerId::AcPower),
            1 => Ok(AcpiTimerId::DcPower),
            _ => Err(TimeAlarmError::InvalidAcpiTimerId),
        }
    }
}

// -------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct AlarmTimerSeconds(u32);
impl AlarmTimerSeconds {
    pub const DISABLED: Self = Self(u32::MAX);
}

impl Default for AlarmTimerSeconds {
    fn default() -> Self {
        Self::DISABLED
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AlarmExpiredWakePolicy(u32);
impl AlarmExpiredWakePolicy {
    #[allow(dead_code)]
    pub const INSTANTLY: Self = Self(0);
    pub const NEVER: Self = Self(u32::MAX);
}

impl Default for AlarmExpiredWakePolicy {
    fn default() -> Self {
        Self::NEVER
    }
}

// -------------------------------------------------

/// Represents an ACPI Time and Alarm Device command.
/// See ACPI Specification 6.4, Section 9.18 "Time and Alarm Device" for details on semantics.
#[rustfmt::skip]
enum AcpiTimeAlarmDeviceCommand {
    // Notably missing from the ACPI spec here is _GCP / 'Get Capabilities'.  It just returns a constant and is expected to be implemented wholly in the ACPI ASL code.

    GetRealTime,                                                // 1: _GRT --> AcpiTimestamp,                 failure: valid bit = 0 in returned timestamp
    SetRealTime(AcpiTimestamp),                                 // 2: _SRT --> u32 (bool),                    failure: u32::MAX
    GetWakeStatus(AcpiTimerId),                                 // 3: _GWS --> u32 (bitmask),                 failure: infallible
    ClearWakeStatus(AcpiTimerId),                               // 4: _CWS --> u32 (bool),                    failure: 1
    SetExpiredTimerPolicy(AcpiTimerId, AlarmExpiredWakePolicy), // 5: _STP --> u32 (bool),                    failure: 1
    SetTimerValue(AcpiTimerId, AlarmTimerSeconds),              // 6: _STV --> u32 (bool),                    failure: 1,
    GetExpiredTimerPolicy(AcpiTimerId),                         // 7: _TIP --> u32 (AlarmExpiredWakePolicy)   failure: infallible
    GetTimerValue(AcpiTimerId),                                 // 8: _TIV --> u32 (AlarmTimerSeconds),       failure: infallible, u32::MAX if disabled

    RespondToInvalidCommand // Not an ACPI method. Used internally to indicate that an invalid command was received, and we must respond with an error asynchronously.
}

impl AcpiTimeAlarmDeviceCommand {
    fn try_from_bytes(bytes: &[u8]) -> Result<Self, TimeAlarmError> {
        // TODO [COMMS] for now, we assume that the message structure is <COMMAND CODE> followed by the ACPI arguments in order as specified in
        //      the ACPI spec https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/09_ACPI-Defined_Devices_and_Device-Specific_Objects/ACPIdefined_Devices_and_DeviceSpecificObjects.html#tiv-timer-values
        //      For example, _STP will be [0x05, <timer identifier>, <policy>]
        //      We need to make sure this is actually how we implement it over eSPI, or adapt to match what eSPI actually sends us.

        const COMMAND_CODE_SIZE_BYTES: usize = core::mem::size_of::<u32>();
        let command_code = u32::from_le_bytes(bytes.try_into()?);

        let bytes = bytes.get(COMMAND_CODE_SIZE_BYTES..)
                                .expect("Should never fail because if there were less than 4 bytes, u32::from_le_bytes would have failed. If there were exactly four bytes, this will return an empty slice, not None.");

        match command_code {
            1 => Ok(AcpiTimeAlarmDeviceCommand::GetRealTime),
            2 => Ok(AcpiTimeAlarmDeviceCommand::SetRealTime(AcpiTimestamp::try_from_bytes(
                bytes,
            )?)),
            _ => {
                let (timer_id, bytes) = AcpiTimerId::try_from_bytes(bytes)?;
                match command_code {
                    3 => Ok(AcpiTimeAlarmDeviceCommand::GetWakeStatus(timer_id)),
                    4 => Ok(AcpiTimeAlarmDeviceCommand::ClearWakeStatus(timer_id)),
                    5 => Ok(AcpiTimeAlarmDeviceCommand::SetExpiredTimerPolicy(
                        timer_id,
                        AlarmExpiredWakePolicy(u32::from_le_bytes(bytes.try_into()?)),
                    )),
                    6 => Ok(AcpiTimeAlarmDeviceCommand::SetTimerValue(
                        timer_id,
                        AlarmTimerSeconds(u32::from_le_bytes(bytes.try_into()?)),
                    )),
                    7 => Ok(AcpiTimeAlarmDeviceCommand::GetExpiredTimerPolicy(timer_id)),
                    8 => Ok(AcpiTimeAlarmDeviceCommand::GetTimerValue(timer_id)),
                    _ => Err(TimeAlarmError::UnknownCommand),
                }
            }
        }
    }
}

enum AcpiTimeAlarmCommandResult {
    /// Used for returning timestamps, i.e. the current time.
    Timestamp(AcpiTimestamp),

    /// Used for returning simple u32 values, such as timer values, wake status bitmasks, etc.
    U32(u32),

    /// The operation succeeded, but there's no data to return.
    Valueless,
}

// -------------------------------------------------

mod time_zone_data {
    use crate::AcpiDaylightSavingsTimeStatus;
    use crate::AcpiTimeZone;
    use crate::NvramStorage;
    use crate::TimeAlarmError;

    pub struct TimeZoneData {
        // Storage used to back the timezone and DST settings.
        storage: &'static mut dyn NvramStorage<'static, u32>,
    }

    #[repr(C)]
    #[derive(bytemuck::Pod, bytemuck::Zeroable, Copy, Clone, Debug)]
    struct RawTimeZoneData {
        tz: i16,
        dst: u8,
        _padding: u8, // padding to make the struct 4 bytes
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

            self.storage.write(bytemuck::cast(representation));
        }

        /// Retreives the current time zone / daylight savings time.
        /// If the stored data is invalid, implying that the NVRAM has never been initialized, defaults to
        /// (AcpiTimeZone::Unknown, AcpiDaylightSavingsTimeStatus::NotObserved).
        ///
        pub fn get_data(&self) -> (AcpiTimeZone, AcpiDaylightSavingsTimeStatus) {
            let representation: RawTimeZoneData = bytemuck::cast(self.storage.read());
            (|| -> Result<(AcpiTimeZone, AcpiDaylightSavingsTimeStatus), TimeAlarmError> {
                Ok((representation.tz.try_into()?, representation.dst.try_into()?))
            })()
            .unwrap_or_else(|_| (AcpiTimeZone::Unknown, AcpiDaylightSavingsTimeStatus::NotObserved))
        }
    }
}
use time_zone_data::TimeZoneData;

// -------------------------------------------------

struct ClockState {
    datetime_clock: &'static mut dyn DatetimeClock,
    tz_data: TimeZoneData,
}

// TODO see if there's some sort of bitfield crate that can make this cleaner
#[derive(Copy, Clone, Debug, Default)]
struct TimerStatus {
    timer_expired: bool,
    timer_triggered_wake: bool,
}

impl From<TimerStatus> for u32 {
    fn from(value: TimerStatus) -> Self {
        let mut result = 0u32;
        if value.timer_expired {
            result |= 0x1;
        }
        if value.timer_triggered_wake {
            result |= 0x2;
        }
        result
    }
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
    acpi_channel: Channel<GlobalRawMutex, (comms::EndpointID, AcpiTimeAlarmDeviceCommand), 10>,

    clock_state: Mutex<GlobalRawMutex, RefCell<ClockState>>,

    // TODO [POWER_SOURCE] signal this whenever the power source changes
    power_source_signal: Signal<GlobalRawMutex, AcpiTimerId>,

    timers: Timers,
}

impl Service {
    // TODO [DYN] if we want to allow taking the HAL traits as concrete types rather than as dyn references, we'll likely need to make this a macro
    //      in order to accommodate the restriction that embassy tasks can't have generic parameters. When we do that, it may be worthwhile to
    //      also investigate ways to take the backing storage as a slice rather than as a bunch of individual references - currently, we can't
    //      take a slice of the array because that would be a slice of trait impls and we need dyn references here to accommodate the constraints
    //      on embassy task implementation.
    //
    pub async fn init(
        service_storage: &'static mut OnceLock<Service>,
        spawner: &embassy_executor::Spawner,
        backing_clock: &'static mut impl DatetimeClock,
        tz_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Result<(), TimeAlarmError> {
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
        });

        // TODO [POWER_SOURCE] we need to subscribe to messages that tell us if we're on AC or DC power so we can decide which alarms to trigger - how do we do that?
        // TODO [POWER_SOURCE] if it's possible to learn which power source is active at init time, we should set that one active rather than defaulting to the AC timer.
        service.timers.ac_timer.start(&service.clock_state, true);
        service.timers.dc_timer.start(&service.clock_state, false);

        comms::register_endpoint(service, &service.endpoint).await?;

        spawner.must_spawn(command_handler_task(service));
        spawner.must_spawn(timer_task(service, AcpiTimerId::AcPower));
        spawner.must_spawn(timer_task(service, AcpiTimerId::DcPower));

        Ok(())
    }

    pub async fn handle_requests(&'static self) {
        loop {
            let acpi_command = self.acpi_channel.receive();
            let power_source_change = self.power_source_signal.wait();

            match select(acpi_command, power_source_change).await {
                Either::First((respond_to_endpoint, acpi_command)) => {
                    const COMMAND_SUCCEEDED: u32 = 1;
                    const COMMAND_FAILED: u32 = 0;

                    let acpi_result = self.handle_acpi_command(acpi_command).await;
                    match acpi_result {
                        Ok(response_payload) => {
                            // TODO [COMMS] is it a problem that we're sending the response in two pieces? If yes, we may need to
                            //      arrange it into a buffer or something. May not be worth solving if we're looking to pivot
                            //      to using something like postcard for this, though.
                            //
                            // TODO [COMMS] it seems like we're sort of conflating wire representation with message representation here -
                            //      is this really how we want to pass messages through the comms system? It seems like it makes it
                            //      harder for other services to send messages to us.  May change with postcard?
                            //
                            self.send_acpi_response(respond_to_endpoint, &COMMAND_SUCCEEDED).await;
                            match response_payload {
                                AcpiTimeAlarmCommandResult::Timestamp(timestamp) => {
                                    self.send_acpi_response(respond_to_endpoint, &timestamp.as_bytes())
                                        .await
                                }
                                AcpiTimeAlarmCommandResult::U32(value) => {
                                    self.send_acpi_response(respond_to_endpoint, &value).await
                                }
                                AcpiTimeAlarmCommandResult::Valueless => (), // nothing more to send
                            }
                        }
                        Err(e) => {
                            error!("Error handling ACPI command: {:?}", e);
                            self.send_acpi_response(respond_to_endpoint, &COMMAND_FAILED).await;
                        }
                    }
                }
                Either::Second(new_power_source) => {
                    info!("Power source changed to {:?}", new_power_source);

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

    pub async fn handle_timer(&'static self, timer_id: AcpiTimerId) {
        let timer = self.timers.get_timer(timer_id);
        loop {
            timer.wait_until_wake(&self.clock_state).await;
            // TODO [SPEC] section 9.18.7 indicates that when a timer expires, both timers have their wake policies reset,
            //      but I can't find any similar rule for the actual timer value - that seems odd to me, verify that's actually how
            //      it's supposed to work
            self.timers
                .get_timer(timer_id.get_other_timer_id())
                .set_timer_wake_policy(&self.clock_state, AlarmExpiredWakePolicy::NEVER);

            // TODO [COMMS] Figure out how to signal a wake event to the host and do that here
        }
    }

    async fn send_acpi_response(&self, destination: comms::EndpointID, response: &impl Any) {
        self.endpoint
            .send(destination, response)
            .await
            .expect("send returns Result<(), Infallible>");
    }

    async fn handle_acpi_command(
        &'static self,
        command: AcpiTimeAlarmDeviceCommand,
    ) -> Result<AcpiTimeAlarmCommandResult, TimeAlarmError> {
        info!("Received Time-Alarm Device command: {:?}", command);
        match command {
            AcpiTimeAlarmDeviceCommand::GetRealTime => self.clock_state.lock(|clock_state| {
                let clock_state = clock_state.borrow();
                let datetime = clock_state.datetime_clock.get_current_datetime()?;
                let (time_zone, dst_status) = clock_state.tz_data.get_data();
                Ok(AcpiTimeAlarmCommandResult::Timestamp(AcpiTimestamp {
                    datetime,
                    time_zone,
                    dst_status,
                }))
            }),
            AcpiTimeAlarmDeviceCommand::SetRealTime(timestamp) => {
                self.clock_state.lock(|clock_state| {
                    let mut clock_state = clock_state.borrow_mut();
                    clock_state.datetime_clock.set_current_datetime(&timestamp.datetime)?;
                    clock_state.tz_data.set_data(timestamp.time_zone, timestamp.dst_status);

                    // TODO [SPEC] the spec is ambiguous on whether or not we should adjust any outstanding timers based on the new time - see if we can find an answer elsewhere
                    Ok(AcpiTimeAlarmCommandResult::Valueless)
                })
            }
            AcpiTimeAlarmDeviceCommand::GetWakeStatus(timer_id) => {
                let status = self.timers.get_timer(timer_id).get_wake_status();
                Ok(AcpiTimeAlarmCommandResult::U32(status.into()))
            }
            AcpiTimeAlarmDeviceCommand::ClearWakeStatus(timer_id) => {
                self.timers.get_timer(timer_id).clear_wake_status();
                Ok(AcpiTimeAlarmCommandResult::Valueless)
            }
            AcpiTimeAlarmDeviceCommand::SetExpiredTimerPolicy(timer_id, timer_policy) => {
                self.timers
                    .get_timer(timer_id)
                    .set_timer_wake_policy(&self.clock_state, timer_policy);
                Ok(AcpiTimeAlarmCommandResult::Valueless)
            }
            AcpiTimeAlarmDeviceCommand::SetTimerValue(timer_id, timer_value) => {
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
                    .set_expiration_time(&self.clock_state, new_expiration_time);
                Ok(AcpiTimeAlarmCommandResult::Valueless)
            }
            AcpiTimeAlarmDeviceCommand::GetExpiredTimerPolicy(timer_id) => Ok(AcpiTimeAlarmCommandResult::U32(
                self.timers.get_timer(timer_id).get_timer_wake_policy().0,
            )),
            AcpiTimeAlarmDeviceCommand::GetTimerValue(timer_id) => {
                let expiration_time = self.timers.get_timer(timer_id).get_expiration_time();

                const ACPI_TIMER_DISABLED: u32 = u32::MAX;
                let timer_wire_format: u32 = match expiration_time {
                    Some(expiration_time) => {
                        let current_time = self
                            .clock_state
                            .lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime())?;

                        expiration_time.to_unix_time_seconds().saturating_sub(current_time.to_unix_time_seconds()).try_into().expect("Per the ACPI spec, timers are communicated in u32 seconds, so this shouldn't be able to overflow")
                    }
                    None => ACPI_TIMER_DISABLED,
                };

                Ok(AcpiTimeAlarmCommandResult::U32(timer_wire_format))
            }

            AcpiTimeAlarmDeviceCommand::RespondToInvalidCommand => Err(TimeAlarmError::InvalidArgument),
        }
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        trace!("Received message at time-alarm-service");

        if let Some(msg) = message.data.get::<AcpiMsgComms>() {
            let buffer_access = msg.payload.borrow();
            let buffer: &[u8] = buffer_access.borrow();

            self.acpi_channel
                .try_send((
                    message.from,
                    AcpiTimeAlarmDeviceCommand::try_from_bytes(&buffer[0..msg.payload_len])
                        .unwrap_or(AcpiTimeAlarmDeviceCommand::RespondToInvalidCommand),
                ))
                .map_err(|_| MailboxDelegateError::BufferFull)?;
            // TODO [COMMS] right now, if pushing the message to the channel fails, the error that we return this gets
            //              discarded by our caller and we have no opportunity to raise a failure. Fixing that probably
            //              requires changes in the mailbox system, so we're ignoring it for now.
            Ok(())
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}

#[embassy_executor::task]
async fn command_handler_task(service: &'static Service) {
    info!("Starting time-alarm service task");
    service.handle_requests().await;
}

#[embassy_executor::task]
async fn timer_task(service: &'static Service, timer_id: AcpiTimerId) {
    info!("Starting time-alarm timer task");
    service.handle_timer(timer_id).await;
}
