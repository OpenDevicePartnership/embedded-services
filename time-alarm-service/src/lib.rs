#![no_std]
// #![allow(unused)] // TODO remove before checkin

use core::any::Any;
use core::array::TryFromSliceError;
use core::borrow::Borrow;
use core::cell::RefCell;
use core::panic;
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

// TODO may want to map other error types here
impl From<TimeAlarmError> for MailboxDelegateError {
    fn from(_error: TimeAlarmError) -> Self {
        MailboxDelegateError::InvalidData
    }
}

impl From<TryFromSliceError> for TimeAlarmError {
    fn from(_error: TryFromSliceError) -> Self {
        TimeAlarmError::InvalidArgument
    }
}

// TODO should an impl like this exist in the MailboxDelegateError crate? For now use map_err
// impl From<TrySendError<AcpiTimeAlarmDeviceCommand>> for MailboxDelegateError {
//     fn from(_error: TrySendError<AcpiTimeAlarmDeviceCommand>) -> Self {
//         match _error {
//             TrySendError::Full(()) => MailboxDelegateError::BufferFull,
//             _ => MailboxDelegateError::Other // TODO is this the right mapping?
//         }
//     }
// }

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
                .try_into()
                .map_err(|_| TimeAlarmError::InvalidArgument)?,
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

// TODO is there some way to do this with an enum that's more readable? I can't figure out how to make it store in
//      a u32 directly and impossible to do Some(u32::MAX) without just adding a panic or whatever. Maybe that's the
//      right thing to do? but then I think we end up spending a byte on the enum discriminant, which seems wasteful?
//      Is there some way to tell the compiler it can get away with inferring discriminant from the value?
//
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
// TODO should these all take an 'endpoint to respond to' parameter? Right now we're assuming that the only thing that will send us commands is the host,
//      which may not be the case?
#[rustfmt::skip]
enum AcpiTimeAlarmDeviceCommand {
    GetCapabilities,                                            // 0: _GCP --> u32 (bitmask),                 failure: infallible
    GetRealTime,                                                // 1: _GRT --> AcpiTimestamp,                 failure: valid bit = 0 in returned timestamp
    SetRealTime(AcpiTimestamp),                                 // 2: _SRT --> u32 (bool),                    failure: u32::MAX
    GetWakeStatus(AcpiTimerId),                                 // 3: _GWS --> u32 (bitmask),                 failure: infallible
    ClearWakeStatus(AcpiTimerId),                               // 4: _CWS --> u32 (bool),                    failure: 1
    SetExpiredTimerPolicy(AcpiTimerId, AlarmExpiredWakePolicy), // 5: _STP --> u32 (bool),                    failure: 1
    SetTimerValue(AcpiTimerId, AlarmTimerSeconds),              // 6: _STV --> u32 (bool),                    failure: 1,
    GetExpiredTimerPolicy(AcpiTimerId),                         // 7: _TIP --> u32 (AlarmExpiredWakePolicy)   failure: infallible
    GetTimerValue(AcpiTimerId),                                 // 8: _TIV --> u32 (AlarmTimerSeconds),       failure: infallible, u32::MAX if disabled
}

impl AcpiTimeAlarmDeviceCommand {
    fn try_from_bytes(bytes: &[u8]) -> Result<Self, TimeAlarmError> {
        // TODO for now, we assume that the message structure is <COMMAND CODE> followed by the ACPI arguments in order as specified in
        //      the ACPI spec https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/09_ACPI-Defined_Devices_and_Device-Specific_Objects/ACPIdefined_Devices_and_DeviceSpecificObjects.html#tiv-timer-values
        //      For example, _STP will be [0x05, <timer identifier>, <policy>]
        //      We need to make sure this is actually how we implement it over eSPI, or adapt to match what eSPI actually sends us.

        const COMMAND_CODE_SIZE_BYTES: usize = core::mem::size_of::<u32>();
        let command_code = u32::from_le_bytes(bytes.try_into()?);

        let bytes = bytes.get(COMMAND_CODE_SIZE_BYTES..)
                                .expect("Should never fail because if there were less than 4 bytes, u32::from_le_bytes would have failed. If there were exactly four bytes, this will return an empty slice, not None.");

        // TODO I feel like these numbers should be associated with the enum somehow, maybe inferred from ordinal position in the enum?
        //      but I don't know if it's possible to do that in Rust.  Figure out if there's a clean way to do it, maybe some to-int
        //      macro or something?
        match command_code {
            0 => Ok(AcpiTimeAlarmDeviceCommand::GetCapabilities), // TODO there's a case to be made that this should just be a hardcoded constant in the ASL - if we do that, may want to renumber
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

// -------------------------------------------------

struct TimeZoneData {
    // Storage used to back the timezone and DST settings.
    // TODO can we achieve this without dyn? I think this would require making the service generic over the clock type, which means no static global SERVICE?
    storage: &'static mut dyn NvramStorage<'static, u32>,
}

// TODO is there a cleaner way to do this sort of bitpacking? I really just want std::bit_cast< struct { int16_t, uint8_t, uint8_t }>
impl TimeZoneData {
    const TZ_MASK: u32 = 0x0000FFFF;
    const DST_MASK: u32 = 0x00FF0000;
    const DST_SHIFT: u32 = 16;

    pub fn new(storage: &'static mut dyn NvramStorage<'static, u32>) -> Self {
        Self { storage }
    }

    pub fn set_data(&mut self, tz: AcpiTimeZone, dst: AcpiDaylightSavingsTimeStatus) {
        let tz_i16: i16 = tz.into();
        let tz_u32 = tz_i16 as u32;

        let dst_u8: u8 = dst.into();
        let dst_u32 = (dst_u8 as u32) << Self::DST_SHIFT;

        self.storage.write(tz_u32 | dst_u32);
    }

    pub fn get_data(&self) -> (AcpiTimeZone, AcpiDaylightSavingsTimeStatus) {
        let tz_u16 = (self.storage.read() & Self::TZ_MASK) as u16;
        let tz_i16 = tz_u16 as i16;
        let tz = AcpiTimeZone::try_from(tz_i16).unwrap_or(AcpiTimeZone::Unknown);

        let dst_u8 = ((self.storage.read() & Self::DST_MASK) >> Self::DST_SHIFT) as u8;
        let dst = AcpiDaylightSavingsTimeStatus::try_from(dst_u8).unwrap_or(AcpiDaylightSavingsTimeStatus::NotObserved);

        (tz, dst)
    }
}

// -------------------------------------------------

struct ClockState {
    // TODO can we achieve this without dyn? I think this would require making the service generic over the clock type,
    //      which means no static global SERVICE / memory allocation for the SERVICE would have to be done by the caller
    //      of init(), which might interfere with the requirement that we can pass static references to it to the comms
    //      system? Need to investigate options for that
    //
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

    // TODO it's a bit unfortunate that we have to pass these in individually like this, but I don't see another way to get compile-time
    //      checking of the number of registers passed in without the experimental split_array_mut - ask if theres' a better way
    fn new(
        ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Self {
        Self {
            ac_timer: Timer::new(true, ac_expiration_storage, ac_policy_storage),
            dc_timer: Timer::new(false, dc_expiration_storage, dc_policy_storage),
        }
    }
}

// -------------------------------------------------

pub struct Service {
    endpoint: comms::Endpoint,

    // ACPI messages from the host are sent through this channel.
    acpi_channel: Channel<GlobalRawMutex, AcpiTimeAlarmDeviceCommand, 10>,

    clock_state: Mutex<GlobalRawMutex, RefCell<ClockState>>,

    power_source_signal: Signal<GlobalRawMutex, AcpiTimerId>, // TODO figure out how to feed this thing

    timers: Timers,
}

// TODO can we template this on the clock/storage type?
impl Service {
    pub fn new(
        backing_clock: &'static mut impl DatetimeClock,
        tz_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
        dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    ) -> Self {
        Service {
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
        }
    }

    pub async fn handle_requests(&self) {
        loop {
            let acpi_command = self.acpi_channel.receive();
            let power_source_change = self.power_source_signal.wait();

            match select(acpi_command, power_source_change).await {
                Either::First(acpi_command) => {
                    self.handle_acpi_command(acpi_command).await.unwrap_or_else(|e| {
                        error!("Error handling ACPI command: {:?}", e);
                        // TODO what should happen if this fails? How do we communicate failure back to the host?
                    });
                }
                Either::Second(new_power_source) => {
                    info!("Power source changed to {:?}", new_power_source);

                    self.timers
                        .get_timer(new_power_source.get_other_timer_id())
                        .set_active(false);
                    self.timers.get_timer(new_power_source).set_active(true);
                }
            }
        }
    }

    pub async fn handle_timer(&self, timer_id: AcpiTimerId) {
        let timer = self.timers.get_timer(timer_id);
        loop {
            timer.wait_until_wake().await;
            // TODO [SPEC_QUESTION] section 9.18.7 indicates that when a timer expires, both timers have their wake policies reset,
            //      but I can't find any similar rule for the actual timer value - that seems odd to me, verify that's actually how
            //      it's supposed to work
            self.timers
                .get_timer(timer_id.get_other_timer_id())
                .set_timer_wake_policy(AlarmExpiredWakePolicy::NEVER);

            todo!("Figure out how to signal a wake event to the host when this timer expires");
        }
    }

    async fn send_acpi_response(&self, response: &impl Any) {
        // TODO right now we're hardcoded to reply to the host, but I think anyone can send us a command - do we need to track
        //      the source endpoint of the command so we can respond to it specifically? What do other services do?
        self.endpoint
            .send(comms::EndpointID::External(comms::External::Host), response)
            .await
            .expect("send returns Result<(), Infallible>");
    }

    async fn handle_acpi_command(&self, command: AcpiTimeAlarmDeviceCommand) -> Result<(), TimeAlarmError> {
        info!("Received Time-Alarm Device command: {:?}", command);
        match command {
            // TODO these all need to return a buffer on error.
            //      The size and shape of that buffer depend on the message type, so we probably can't punt to the caller unless we want to do the translation in the ASL? That seems cleaner to me, but maybe there's some reason not to do that?
            AcpiTimeAlarmDeviceCommand::GetCapabilities => {
                todo!(
                    "implement or remove GetCapabilities - if implemented, it'd return a constant, so there's a case to be made that it belongs wholly in the ASL"
                );
            }
            AcpiTimeAlarmDeviceCommand::GetRealTime => {
                let time = self.clock_state.lock(|clock_state| {
                    let clock_state = clock_state.borrow();
                    match clock_state.datetime_clock.get_current_datetime() {
                        // TODO figure out why type inference doesn't work with map_err and the ? operator
                        Ok(datetime) => {
                            let (time_zone, dst_status) = clock_state.tz_data.get_data();
                            Ok(AcpiTimestamp {
                                datetime,
                                time_zone,
                                dst_status,
                            })
                        }
                        Err(e) => Err(TimeAlarmError::ClockError(e)),
                    }
                })?;

                self.send_acpi_response(&time.as_bytes()).await;
                // TODO is this sufficient, or do we also need a 'success' / 'EOM' packet or something?

                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::SetRealTime(timestamp) => {
                self.clock_state.lock(|clock_state| {
                    let mut clock_state = clock_state.borrow_mut();
                    clock_state
                        .datetime_clock
                        .set_current_datetime(&timestamp.datetime)
                        .map_err(TimeAlarmError::ClockError)?;
                    clock_state.tz_data.set_data(timestamp.time_zone, timestamp.dst_status);

                    // TODO do we need to return a success code over espi?
                    // TODO do we need to adjust the timers based on the new time? check with ACPI spec
                    Ok(())
                })
            }
            AcpiTimeAlarmDeviceCommand::GetWakeStatus(timer_id) => {
                let status = self.timers.get_timer(timer_id).get_wake_status();
                let packed_status: u32 = status.into();
                self.send_acpi_response(&packed_status.to_le_bytes()).await;
                // TODO is this sufficient, or do we also need a 'success' / 'EOM' packet or something?

                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::ClearWakeStatus(timer_id) => {
                self.timers.get_timer(timer_id).clear_wake_status();
                // TODO do we need to return a success code over espi?
                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::SetExpiredTimerPolicy(timer_id, timer_policy) => {
                self.timers.get_timer(timer_id).set_timer_wake_policy(timer_policy);
                // TODO do we need to return a success code over espi?
                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::SetTimerValue(timer_id, timer_value) => {
                let new_expiration_time = match timer_value {
                    AlarmTimerSeconds::DISABLED => None,
                    AlarmTimerSeconds(secs) => {
                        let current_time = self.clock_state.lock(|clock_state| {
                            let clock_state = clock_state.borrow();
                            clock_state
                                .datetime_clock
                                .get_current_datetime()
                                .map_err(TimeAlarmError::ClockError)
                        })?;

                        let expiration_time =
                            Datetime::from_unix_time_seconds(current_time.to_unix_time_seconds() + u64::from(secs)); // TODO why doesn't into() work here?
                        Some(expiration_time)
                    }
                };

                self.timers.get_timer(timer_id).set_expiration_time(new_expiration_time);
                // TODO do we need to return a success code over espi?

                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::GetExpiredTimerPolicy(timer_id) => {
                let wake_policy = self.timers.get_timer(timer_id).get_timer_wake_policy();
                self.send_acpi_response(&wake_policy.0.to_le_bytes()).await;
                // TODO is this sufficient, or do we also need a 'success' / 'EOM' packet or something?

                Ok(())
            }
            AcpiTimeAlarmDeviceCommand::GetTimerValue(timer_id) => {
                let expiration_time = self.timers.get_timer(timer_id).get_expiration_time();

                const ACPI_TIMER_DISABLED: u32 = u32::MAX;
                let timer_wire_format: u32 = match expiration_time {
                    Some(expiration_time) => {
                        let current_time = self.clock_state.lock(|clock_state| {
                            let clock_state = clock_state.borrow();
                            clock_state
                                .datetime_clock
                                .get_current_datetime()
                                .map_err(TimeAlarmError::ClockError)
                        })?;

                        expiration_time.to_unix_time_seconds().saturating_sub(current_time.to_unix_time_seconds()).try_into().expect("Per the ACPI spec, timers are communicated in u32 seconds, so this shouldn't be able to overflow")
                    }
                    None => ACPI_TIMER_DISABLED,
                };

                self.send_acpi_response(&timer_wire_format.to_le_bytes()).await;
                // TODO is this sufficient, or do we also need a 'success' / 'EOM' packet or something?

                Ok(())
            }
        }
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, _message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        trace!("Received message at time-alarm-service");

        if let Some(msg) = _message.data.get::<AcpiMsgComms>() {
            let buffer_access = msg.payload.borrow();
            let buffer: &[u8] = buffer_access.borrow();

            // TODO right now, if this fails, what happens? The TAD spec has different ways of reporting failure depending on the command, do we need to handle that here or is that in the ASL?
            // TODO what is supposed to happen if there's e.g. an invalid timestamp?  Is returning an error here sufficient, or do we need to send some kind of error response back to the host?
            self.acpi_channel
                .try_send(AcpiTimeAlarmDeviceCommand::try_from_bytes(&buffer[0..msg.payload_len])?)
                .map_err(|_| MailboxDelegateError::BufferFull)?;
            Ok(())
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}

static SERVICE: OnceLock<Service> = OnceLock::new(); // TODO is this really what we want? I'd love to have init return an instance instead, that would let us template over the clock type... unclear how that would work with register_endpoint, though

// TODO figure out if there's a cleaner way to pass a bunch of these storage instances in without having a ton of parameters - tried taking a slice but hit some weird type issues that I'm punting on for now
pub async fn init(
    spawner: &embassy_executor::Spawner,
    backing_clock: &'static mut impl DatetimeClock,
    tz_storage: &'static mut dyn NvramStorage<'static, u32>,
    ac_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
    ac_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
    dc_expiration_storage: &'static mut dyn NvramStorage<'static, u32>,
    dc_policy_storage: &'static mut dyn NvramStorage<'static, u32>,
) -> Result<(), TimeAlarmError> {
    info!("Starting time-alarm service task");

    let service = SERVICE.get_or_init(|| {
        Service::new(
            backing_clock,
            tz_storage,
            ac_expiration_storage,
            ac_policy_storage,
            dc_expiration_storage,
            dc_policy_storage,
        )
    });

    comms::register_endpoint(service, &service.endpoint)
        .await
        .map_err(|_| {
            error!("Failed to register time-alarm service endpoint");
            TimeAlarmError::DoubleInitError // TODO if we impl from on error type, tear this out
        })?;

    // TODO we need to subscribe to messages that tell us if we're on AC or DC power so we can decide which alarms to trigger - how do we do that?

    spawner.must_spawn(command_handler_task());
    spawner.must_spawn(timer_task(AcpiTimerId::AcPower));
    spawner.must_spawn(timer_task(AcpiTimerId::DcPower));

    Ok(())
}

#[embassy_executor::task]
async fn command_handler_task() {
    info!("Starting time-alarm service task");
    let service = SERVICE.get_or_init(|| panic!("should have already been initialized by init()"));

    service.handle_requests().await;
}

#[embassy_executor::task]
async fn timer_task(timer_id: AcpiTimerId) {
    info!("Starting time-alarm timer task");
    let service = SERVICE.get_or_init(|| panic!("should have already been initialized by init()"));

    service.handle_timer(timer_id).await;
}

pub fn get_current_datetime() -> Option<Datetime> {
    SERVICE
        .try_get()?
        .clock_state
        .lock(|clock_state| clock_state.borrow().datetime_clock.get_current_datetime().ok())
}
