use embedded_mcu_hal::time::{Datetime, DatetimeFields};
use embedded_services::hid_report;
use embedded_services::relay::hid::ReportId;
use embedded_services::relay::hid::reports;
use time_alarm_service_interface::{AcpiTimeZone, AcpiTimeZoneOffset};

#[allow(dead_code)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum TimestampConversionError {
    Report(reports::ReportError),
    Datetime(embedded_mcu_hal::time::DatetimeError),
    DatetimeClock(embedded_mcu_hal::time::DatetimeClockError),
}

impl From<reports::ReportError> for TimestampConversionError {
    fn from(err: reports::ReportError) -> Self {
        Self::Report(err)
    }
}
impl From<embedded_mcu_hal::time::DatetimeError> for TimestampConversionError {
    fn from(err: embedded_mcu_hal::time::DatetimeError) -> Self {
        Self::Datetime(err)
    }
}
impl From<embedded_mcu_hal::time::DatetimeClockError> for TimestampConversionError {
    fn from(err: embedded_mcu_hal::time::DatetimeClockError) -> Self {
        Self::DatetimeClock(err)
    }
}

// HID Usage Tables: 1.7.0
// Descriptor size: 367 (bytes)
// +----------+---------+-------------------+
// | ReportId | Kind    | ReportSizeInBytes |
// +----------+---------+-------------------+
// |        1 | Input   |                10 |
// +----------+---------+-------------------+
// |        1 | Output  |                 4 |
// +----------+---------+-------------------+
// |        1 | Feature |                 1 |
// +----------+---------+-------------------+
// |        2 | Input   |                 9 |
// +----------+---------+-------------------+
// |        2 | Output  |                 4 |
// +----------+---------+-------------------+
// |        3 | Output  |                 1 |
// +----------+---------+-------------------+
// |        4 | Output  |                 8 |
// +----------+---------+-------------------+

// -------- INPUT REPORTS --------

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
pub(crate) enum InputReportId {
    GetAlarm = 1,
    GetTime = 2,
}

impl TryFrom<ReportId> for InputReportId {
    type Error = num_enum::TryFromPrimitiveError<InputReportId>;

    fn try_from(value: ReportId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

hid_report! {
    pub(crate) struct GetAlarmReport {
        ac_timer: u32 => 31,
        _padding1: u8 => 1,
        dc_timer: u32 => 31,
        power_source_change_debounce_seconds: u8 => 6,
        current_state: u8 => 3,
        vendor_current_state: u8 => 4,
        _padding: u8 => 4
        // Define the fields for the GetAlarmReport here
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub(crate) enum AlarmCurrentState {
    Cleared = 1,
    Failed = 2,
    Running = 3,
    Expired = 4,
    Signaled = 5,
}

impl GetAlarmReport {
    pub(crate) fn new(
        ac_timer: time_alarm_service_interface::AlarmTimerSeconds,
        dc_timer: time_alarm_service_interface::AlarmTimerSeconds,
        power_policy: time_alarm_service_interface::AlarmExpiredWakePolicy,
        current_state: AlarmCurrentState,
        vendor_current_state: u8,
    ) -> Self {
        Self {
            ac_timer: convert_timer_to_wire(ac_timer),
            _padding1: 0,
            dc_timer: convert_timer_to_wire(dc_timer),
            power_source_change_debounce_seconds: convert_policy_to_wire(power_policy),
            current_state: current_state.into(),
            vendor_current_state,
            _padding: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub(crate) enum TimeCurrentState {
    Failed = 1,
    Running = 2,
}

hid_report! {
    pub(crate) struct GetTimeReport {
        /// 1900 - 9999
        year: u16 => 14,
        /// 1 - 12
        month: u8 => 4,
        /// 1 - 31
        day: u8 => 5,
        /// 0 - 23
        hour: u8 => 5,
        /// 0 - 59
        minute: u8 => 6,
        /// 0 - 59
        second: u8 => 6,
        /// 0 - 999
        millisecond: u16 => 10,
        /// -1440 - 1440, minutes from UTC
        time_zone: i16 => 12,
        dst_observed: bool => 1,
        dst_active: bool => 1,
        current_state: u8 => 2,
        vendor_current_state: u8 => 4,
        _reserved: u8 => 2
    }
}

impl GetTimeReport {
    pub(crate) fn new(
        ts: time_alarm_service_interface::AcpiTimestamp,
        current_state: TimeCurrentState,
        vendor_state: u8,
    ) -> Self {
        Self {
            year: ts.datetime.year(),
            month: ts.datetime.month().into(),
            day: ts.datetime.day(),
            hour: ts.datetime.hour(),
            minute: ts.datetime.minute(),
            second: ts.datetime.second(),
            millisecond: (ts.datetime.nanoseconds() / 1_000_000) as u16,
            time_zone: time_zone_to_wire(ts.time_zone),
            dst_observed: matches!(
                ts.dst_status,
                time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotAdjusted
                    | time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted
            ),
            dst_active: matches!(
                ts.dst_status,
                time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted
            ),
            current_state: current_state.into(),
            vendor_current_state: vendor_state,
            _reserved: 0,
        }
    }

    /// Report used when the current time is unavailable; the time fields carry no meaning.
    pub(crate) fn failed(vendor_state: u8) -> Self {
        Self {
            current_state: TimeCurrentState::Failed.into(),
            vendor_current_state: vendor_state,
            ..Default::default()
        }
    }
}

// -------- OUTPUT REPORTS --------

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum OutputReportId {
    SetAcAlarm = 1,
    SetDcAlarm = 2,
    SetDebounce = 3,
    SetTime = 4,
}

impl TryFrom<ReportId> for OutputReportId {
    type Error = num_enum::TryFromPrimitiveError<OutputReportId>;

    fn try_from(value: ReportId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

hid_report! {
    pub(crate) struct SetAlarmReport {
        pub timer_seconds: u32 => 31,
        _padding: u8 => 1
    }
}

hid_report! {
    pub(crate) struct SetDebounceReport {
        pub power_source_change_debounce_seconds: u8 => 6,
        _padding: u8 => 2
    }
}

/// Converts a timer value in seconds to an `AlarmTimerSeconds` enum, mapping the HID null value to
/// disabled. See [`TIMER_NULL`]; if we change the supported logical value range, we'll need to
/// update this too.
pub(crate) fn convert_timer(seconds: u32) -> time_alarm_service_interface::AlarmTimerSeconds {
    if seconds == TIMER_NULL {
        time_alarm_service_interface::AlarmTimerSeconds::DISABLED
    } else {
        time_alarm_service_interface::AlarmTimerSeconds(seconds)
    }
}

/// A value in the physical range but out of the logical range for our timer. Must agree with report descriptor.
const TIMER_NULL: u32 = 0;

/// LogicalMaximum of the alarm timer items in our report descriptor; also the widest value the
/// 31-bit timer fields can hold.
const TIMER_LOGICAL_MIN: u32 = 1;
const TIMER_LOGICAL_MAX: u32 = 0x7FFF_FFFF;

/// LogicalMaximum of the power source change debounce items in our report descriptor.
const DEBOUNCE_LOGICAL_MIN: u32 = 1;
const DEBOUNCE_LOGICAL_MAX: u32 = 60;

/// Inverse of [`convert_timer`]: a disabled timer is reported as the null value, and values the
/// descriptor can't express are clamped rather than failing the whole report.
fn convert_timer_to_wire(timer: time_alarm_service_interface::AlarmTimerSeconds) -> u32 {
    if timer == time_alarm_service_interface::AlarmTimerSeconds::DISABLED {
        TIMER_NULL
    } else if timer == time_alarm_service_interface::AlarmTimerSeconds(0) {
        // TODO - there's currently a disagreement between HID and ACPI on what a logical value of "0" means.
        //        ACPI says it means "this timer is expired" but HID says it's not a valid value, which in
        //        effect means "this timer is disabled".
        //        For now, we treat it as disabled, but there may be a case to be made that it should mean
        //        the same thing as in ACPI.
        TIMER_NULL
    } else {
        timer.0.clamp(TIMER_LOGICAL_MIN, TIMER_LOGICAL_MAX)
    }
}

fn convert_policy_to_wire(policy: time_alarm_service_interface::AlarmExpiredWakePolicy) -> u8 {
    match policy {
        time_alarm_service_interface::AlarmExpiredWakePolicy::INSTANTLY => 0,
        // TODO "never" isn't expressible with the current HID interface; need to circle back with time and hid folks
        //      on if this was a deliberate design decision or an oversight.  For now, report it as our logical max.
        time_alarm_service_interface::AlarmExpiredWakePolicy::NEVER => DEBOUNCE_LOGICAL_MAX as u8,
        time_alarm_service_interface::AlarmExpiredWakePolicy(seconds) => {
            seconds.clamp(DEBOUNCE_LOGICAL_MIN, DEBOUNCE_LOGICAL_MAX) as u8
        }
    }
}

/// Inverse of [`convert_policy_to_wire`]. The null value means the host isn't asking for a minimum
/// expiration, so we wake instantly; NEVER has no wire encoding and so can't be round-tripped.
pub(crate) fn convert_policy(seconds: u8) -> time_alarm_service_interface::AlarmExpiredWakePolicy {
    match u32::from(seconds) {
        seconds @ DEBOUNCE_LOGICAL_MIN..=DEBOUNCE_LOGICAL_MAX => {
            time_alarm_service_interface::AlarmExpiredWakePolicy(seconds)
        }
        _ => time_alarm_service_interface::AlarmExpiredWakePolicy::INSTANTLY,
    }
}

const NULL_HID_TIME_ZONE: i16 = 2047;
fn time_zone_to_wire(time_zone: AcpiTimeZone) -> i16 {
    match time_zone {
        AcpiTimeZone::Unknown => NULL_HID_TIME_ZONE,
        AcpiTimeZone::MinutesFromUtc(offset) => offset.minutes_from_utc(),
    }
}

fn time_zone_from_wire(time_zone: i16) -> AcpiTimeZone {
    match AcpiTimeZoneOffset::new(time_zone) {
        Ok(offset) => AcpiTimeZone::MinutesFromUtc(offset),
        Err(_) => AcpiTimeZone::Unknown,
    }
}

hid_report! {
    pub(crate) struct SetTimeReport {
        /// 1900 - 9999
        pub year: u16 => 14,
        /// 1 - 12
        pub month: u8 => 4,
        /// 1 - 31
        pub day: u8 => 5,
        /// 0 - 23
        pub hour: u8 => 5,
        /// 0 - 59
        pub minute: u8 => 6,
        /// 0 - 59
        pub second: u8 => 6,
        /// 0 - 999
        pub millisecond: u16 => 10,
        /// -1440 - 1440, minutes from UTC
        pub time_zone: i16 => 12,
        pub dst_observed: bool => 1,
        pub dst_active: bool => 1,
    }
}

impl TryFrom<SetTimeReport> for time_alarm_service_interface::AcpiTimestamp {
    type Error = TimestampConversionError;
    fn try_from(report: SetTimeReport) -> Result<Self, Self::Error> {
        Ok(Self {
            datetime: Datetime::new(DatetimeFields {
                year: report.year,
                month: report
                    .month
                    .try_into()
                    .map_err(|_| TimestampConversionError::Datetime(embedded_mcu_hal::time::DatetimeError::Month))?,
                day: report.day,
                hour: report.hour,
                minute: report.minute,
                second: report.second,
                nanosecond: report.millisecond as u32 * 1_000_000,
            })?,
            time_zone: time_zone_from_wire(report.time_zone),
            dst_status: match (report.dst_observed, report.dst_active) {
                (false, false) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotObserved,
                (true, false) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotAdjusted,
                (true, true) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted,
                (false, true) => {
                    return Err(TimestampConversionError::Report(reports::ReportError::ValueOutOfRange));
                }
            },
        })
    }
}

// -------- FEATURE REPORTS --------

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
pub(crate) enum FeatureReportId {
    Capabilities = 1,
}

impl TryFrom<ReportId> for FeatureReportId {
    type Error = num_enum::TryFromPrimitiveError<FeatureReportId>;

    fn try_from(value: ReportId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
pub(crate) enum PowerState {
    S3 = 1,
    S4 = 2,
    S5 = 3,
}

hid_report! {
    pub(crate) struct CapabilitiesFeatureReport {
        pub power_state: u8 => 2, // Type: PowerState
        _padding: u8 => 6
    }
}

impl CapabilitiesFeatureReport {
    pub(crate) fn new(power_state: PowerState) -> Self {
        Self {
            power_state: power_state.into(),
            _padding: 0,
        }
    }
}

// Generated from Waratah - don't hand-edit this. Instead, modify the .wara file and rerun Waratah.
// See https://github.com/microsoft/hidtools more information on Waratah.
// Modifications to this require alterations to the above types to match.
#[rustfmt::skip]
pub(crate) const TIME_ALARM_HID_DESCRIPTOR: &[u8] = &[
    0x05, 0x01,                      // UsagePage(Generic Desktop[0x0001])
    0x09, 0x14,                      // UsageId(System Wake Timer and Real Time Clock[0x0014])
    0xA1, 0x01,                      // Collection(Application)
    0x85, 0x01,                      //     ReportId(1)
    0x09, 0xF3,                      //     UsageId(Lowest System Wakeable Power State[0x00F3])
    0xA1, 0x02,                      //     Collection(Logical)
    0x19, 0xF6,                      //         UsageIdMin(S3[0x00F6])
    0x29, 0xF8,                      //         UsageIdMax(S5[0x00F8])
    0x15, 0x01,                      //         LogicalMinimum(1)
    0x25, 0x03,                      //         LogicalMaximum(3)
    0x95, 0x01,                      //         ReportCount(1)
    0x75, 0x02,                      //         ReportSize(2)
    0xB1, 0x00,                      //         Feature(Data, Array, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0xC0,                            //     EndCollection()
    0x75, 0x06,                      //     ReportSize(6)
    0xB1, 0x03,                      //     Feature(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0xF0,                      //     UsageId(Timer Expiration: External Power[0x00F0])
    0x66, 0x01, 0x10,                //     Unit('second', SiLinear, Seconds:1)
    0x27, 0xFF, 0xFF, 0xFF, 0x7F,    //     LogicalMaximum(2,147,483,647)
    0x75, 0x1F,                      //     ReportSize(31)
    0x91, 0x42,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, NonVolatile, BitField)
    0x75, 0x01,                      //     ReportSize(1)
    0x91, 0x03,                      //     Output(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x85, 0x02,                      //     ReportId(2)
    0x09, 0xF1,                      //     UsageId(Timer Expiration: Internal Power[0x00F1])
    0x75, 0x1F,                      //     ReportSize(31)
    0x91, 0x42,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, NonVolatile, BitField)
    0x75, 0x01,                      //     ReportSize(1)
    0x91, 0x03,                      //     Output(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x85, 0x03,                      //     ReportId(3)
    0x09, 0xF2,                      //     UsageId(Power Source Change Minimum Expiration[0x00F2])
    0x25, 0x3C,                      //     LogicalMaximum(60)
    0x75, 0x06,                      //     ReportSize(6)
    0x91, 0x42,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, NonVolatile, BitField)
    0x75, 0x02,                      //     ReportSize(2)
    0x91, 0x03,                      //     Output(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x85, 0x01,                      //     ReportId(1)
    0x09, 0xF0,                      //     UsageId(Timer Expiration: External Power[0x00F0])
    0x27, 0xFF, 0xFF, 0xFF, 0x7F,    //     LogicalMaximum(2,147,483,647)
    0x75, 0x1F,                      //     ReportSize(31)
    0x81, 0x42,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, BitField)
    0x75, 0x01,                      //     ReportSize(1)
    0x81, 0x03,                      //     Input(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0xF1,                      //     UsageId(Timer Expiration: Internal Power[0x00F1])
    0x75, 0x1F,                      //     ReportSize(31)
    0x81, 0x42,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, BitField)
    0x09, 0xF2,                      //     UsageId(Power Source Change Minimum Expiration[0x00F2])
    0x25, 0x3C,                      //     LogicalMaximum(60)
    0x75, 0x06,                      //     ReportSize(6)
    0x81, 0x42,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, BitField)
    0x05, 0x06,                      //     UsagePage(Generic Device Controls[0x0006])
    0x09, 0x51,                      //     UsageId(Current State[0x0051])
    0xA1, 0x02,                      //     Collection(Logical)
    0x09, 0x52,                      //         UsageId(Cleared[0x0052])
    0x09, 0x53,                      //         UsageId(Failed[0x0053])
    0x09, 0x54,                      //         UsageId(Running[0x0054])
    0x09, 0x55,                      //         UsageId(Expired[0x0055])
    0x09, 0x56,                      //         UsageId(Signaled[0x0056])
    0x25, 0x05,                      //         LogicalMaximum(5)
    0x75, 0x03,                      //         ReportSize(3)
    0x81, 0x00,                      //         Input(Data, Array, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0xC0,                            //     EndCollection()
    0x09, 0x50,                      //     UsageId(Vendor Current State[0x0050])
    0x65, 0x00,                      //     Unit(None)
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x0F,                      //     LogicalMaximum(15)
    0x75, 0x04,                      //     ReportSize(4)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x81, 0x03,                      //     Input(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x85, 0x04,                      //     ReportId(4)
    0x05, 0x13,                      //     UsagePage(Time and Date[0x0013])
    0x09, 0x01,                      //     UsageId(Year[0x0001])
    0x16, 0x6C, 0x07,                //     LogicalMinimum(1,900)
    0x26, 0x0F, 0x27,                //     LogicalMaximum(9,999)
    0x75, 0x0E,                      //     ReportSize(14)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x02,                      //     UsageId(Month[0x0002])
    0x15, 0x01,                      //     LogicalMinimum(1)
    0x25, 0x0C,                      //     LogicalMaximum(12)
    0x75, 0x04,                      //     ReportSize(4)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x03,                      //     UsageId(Day[0x0003])
    0x25, 0x1F,                      //     LogicalMaximum(31)
    0x75, 0x05,                      //     ReportSize(5)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x04,                      //     UsageId(Hour[0x0004])
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x17,                      //     LogicalMaximum(23)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x05,                      //     UsageId(Minute[0x0005])
    0x09, 0x06,                      //     UsageId(Second[0x0006])
    0x25, 0x3B,                      //     LogicalMaximum(59)
    0x95, 0x02,                      //     ReportCount(2)
    0x75, 0x06,                      //     ReportSize(6)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x07,                      //     UsageId(Millisecond[0x0007])
    0x26, 0xE7, 0x03,                //     LogicalMaximum(999)
    0x95, 0x01,                      //     ReportCount(1)
    0x75, 0x0A,                      //     ReportSize(10)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x09, 0x10,                      //     UsageId(Time Zone Offset From UTC[0x0010])
    0x16, 0x60, 0xFA,                //     LogicalMinimum(-1,440)
    0x26, 0xA0, 0x05,                //     LogicalMaximum(1,440)
    0x75, 0x0C,                      //     ReportSize(12)
    0x91, 0x42,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, NonVolatile, BitField)
    0x09, 0x11,                      //     UsageId(Daylight Savings Time Observed[0x0011])
    0x09, 0x12,                      //     UsageId(Daylight Savings Time Active[0x0012])
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x01,                      //     LogicalMaximum(1)
    0x95, 0x02,                      //     ReportCount(2)
    0x75, 0x01,                      //     ReportSize(1)
    0x91, 0x02,                      //     Output(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, NonVolatile, BitField)
    0x85, 0x02,                      //     ReportId(2)
    0x09, 0x01,                      //     UsageId(Year[0x0001])
    0x16, 0x6C, 0x07,                //     LogicalMinimum(1,900)
    0x26, 0x0F, 0x27,                //     LogicalMaximum(9,999)
    0x95, 0x01,                      //     ReportCount(1)
    0x75, 0x0E,                      //     ReportSize(14)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x02,                      //     UsageId(Month[0x0002])
    0x15, 0x01,                      //     LogicalMinimum(1)
    0x25, 0x0C,                      //     LogicalMaximum(12)
    0x75, 0x04,                      //     ReportSize(4)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x03,                      //     UsageId(Day[0x0003])
    0x25, 0x1F,                      //     LogicalMaximum(31)
    0x75, 0x05,                      //     ReportSize(5)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x04,                      //     UsageId(Hour[0x0004])
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x17,                      //     LogicalMaximum(23)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x05,                      //     UsageId(Minute[0x0005])
    0x09, 0x06,                      //     UsageId(Second[0x0006])
    0x25, 0x3B,                      //     LogicalMaximum(59)
    0x95, 0x02,                      //     ReportCount(2)
    0x75, 0x06,                      //     ReportSize(6)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x07,                      //     UsageId(Millisecond[0x0007])
    0x26, 0xE7, 0x03,                //     LogicalMaximum(999)
    0x95, 0x01,                      //     ReportCount(1)
    0x75, 0x0A,                      //     ReportSize(10)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x09, 0x10,                      //     UsageId(Time Zone Offset From UTC[0x0010])
    0x16, 0x60, 0xFA,                //     LogicalMinimum(-1,440)
    0x26, 0xA0, 0x05,                //     LogicalMaximum(1,440)
    0x75, 0x0C,                      //     ReportSize(12)
    0x81, 0x42,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NullState, BitField)
    0x09, 0x11,                      //     UsageId(Daylight Savings Time Observed[0x0011])
    0x09, 0x12,                      //     UsageId(Daylight Savings Time Active[0x0012])
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x01,                      //     LogicalMaximum(1)
    0x95, 0x02,                      //     ReportCount(2)
    0x75, 0x01,                      //     ReportSize(1)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x05, 0x06,                      //     UsagePage(Generic Device Controls[0x0006])
    0x09, 0x51,                      //     UsageId(Current State[0x0051])
    0xA1, 0x02,                      //     Collection(Logical)
    0x09, 0x53,                      //         UsageId(Failed[0x0053])
    0x09, 0x54,                      //         UsageId(Running[0x0054])
    0x15, 0x01,                      //         LogicalMinimum(1)
    0x25, 0x02,                      //         LogicalMaximum(2)
    0x95, 0x01,                      //         ReportCount(1)
    0x75, 0x02,                      //         ReportSize(2)
    0x81, 0x00,                      //         Input(Data, Array, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0xC0,                            //     EndCollection()
    0x09, 0x50,                      //     UsageId(Vendor Current State[0x0050])
    0x15, 0x00,                      //     LogicalMinimum(0)
    0x25, 0x0F,                      //     LogicalMaximum(15)
    0x75, 0x04,                      //     ReportSize(4)
    0x81, 0x02,                      //     Input(Data, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0x75, 0x02,                      //     ReportSize(2)
    0x81, 0x03,                      //     Input(Constant, Variable, Absolute, NoWrap, Linear, PreferredState, NoNullPosition, BitField)
    0xC0,                            // EndCollection()
];
