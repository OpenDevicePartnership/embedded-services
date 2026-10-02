use bitfield::bitfield;
use embedded_mcu_hal::time::{Datetime, DatetimeFields};
use embedded_services::relay::hid::ReportId;
use time_alarm_service_interface::AcpiTimeZone;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum ReportError {
    InvalidLength,
    ValueOutOfRange,
    InvalidDatetime,
}

impl From<embedded_mcu_hal::time::DatetimeError> for ReportError {
    fn from(_: embedded_mcu_hal::time::DatetimeError) -> Self {
        Self::InvalidDatetime
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

/// A value in the physical range but out of the logical range for our timer. Must agree with report descriptor.
const TIMER_NULL: u32 = 0;

/// LogicalMaximum of the alarm timer items in our report descriptor; also the widest value the
/// 31-bit timer fields can hold.
const TIMER_LOGICAL_MIN: u32 = 1;
const TIMER_LOGICAL_MAX: u32 = 0x7FFF_FFFF;

#[derive(Clone, Copy, Debug, PartialEq)]
struct HidTimerSeconds(u32);

impl From<u32> for HidTimerSeconds {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<time_alarm_service_interface::AlarmTimerSeconds> for HidTimerSeconds {
    fn from(value: time_alarm_service_interface::AlarmTimerSeconds) -> Self {
        Self(match value {
            time_alarm_service_interface::AlarmTimerSeconds::DISABLED => TIMER_NULL,

            // TODO - there's currently a disagreement between HID and ACPI on what a logical value of "0" means.
            //        ACPI says it means "this timer is expired" but HID says it's not a valid value, which in
            //        effect means "this timer is disabled".
            //        For now, we treat it as disabled, but there may be a case to be made that it should mean
            //        the same thing as in ACPI.
            time_alarm_service_interface::AlarmTimerSeconds(0) => TIMER_NULL,
            time_alarm_service_interface::AlarmTimerSeconds(seconds) => {
                seconds.clamp(TIMER_LOGICAL_MIN, TIMER_LOGICAL_MAX)
            }
        })
    }
}

impl From<HidTimerSeconds> for u32 {
    fn from(value: HidTimerSeconds) -> Self {
        value.0
    }
}

impl From<HidTimerSeconds> for time_alarm_service_interface::AlarmTimerSeconds {
    fn from(value: HidTimerSeconds) -> Self {
        if value.0 == TIMER_NULL {
            Self::DISABLED
        } else {
            Self(value.0)
        }
    }
}

/// LogicalMaximum of the power source change debounce items in our report descriptor.
const DEBOUNCE_LOGICAL_MIN: u32 = 1;
const DEBOUNCE_LOGICAL_MAX: u32 = 60;

#[derive(Clone, Copy, Debug, PartialEq)]
struct HidWakePolicy(u8);

impl From<u8> for HidWakePolicy {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<time_alarm_service_interface::AlarmExpiredWakePolicy> for HidWakePolicy {
    fn from(value: time_alarm_service_interface::AlarmExpiredWakePolicy) -> Self {
        Self(match value {
            time_alarm_service_interface::AlarmExpiredWakePolicy::INSTANTLY => 0,
            // TODO "never" isn't expressible with the current HID interface; need to circle back with time and hid folks
            //      on if this was a deliberate design decision or an oversight.  For now, report it as our logical max.
            time_alarm_service_interface::AlarmExpiredWakePolicy::NEVER => DEBOUNCE_LOGICAL_MAX as u8,
            time_alarm_service_interface::AlarmExpiredWakePolicy(seconds) => {
                seconds.clamp(DEBOUNCE_LOGICAL_MIN, DEBOUNCE_LOGICAL_MAX) as u8
            }
        })
    }
}

impl From<HidWakePolicy> for u8 {
    fn from(value: HidWakePolicy) -> Self {
        value.0
    }
}

impl From<HidWakePolicy> for time_alarm_service_interface::AlarmExpiredWakePolicy {
    fn from(value: HidWakePolicy) -> Self {
        // The HID null value means no minimum delay; other values outside the logical range are
        // treated the same way because they carry no valid policy.
        match u32::from(value.0) {
            seconds @ DEBOUNCE_LOGICAL_MIN..=DEBOUNCE_LOGICAL_MAX => Self(seconds),
            _ => Self::INSTANTLY,
        }
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct GetAlarmReport([u8]);
    impl Debug;
    u32, from into HidTimerSeconds, ac_timer, set_ac_timer: 30, 0;
    u32, from into HidTimerSeconds, dc_timer, set_dc_timer: 62, 32;
    u8, from into HidWakePolicy, power_source_change_debounce, set_power_source_change_debounce: 68, 63;
    u8, current_state, set_current_state: 71, 69;
    u8, vendor_current_state, set_vendor_current_state: 75, 72;
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

impl GetAlarmReport<[u8; 10]> {
    fn new(
        ac_timer: time_alarm_service_interface::AlarmTimerSeconds,
        dc_timer: time_alarm_service_interface::AlarmTimerSeconds,
        power_policy: time_alarm_service_interface::AlarmExpiredWakePolicy,
        current_state: AlarmCurrentState,
        vendor_current_state: u8,
    ) -> Self {
        let mut report = Self([0; 10]);
        report.set_ac_timer(ac_timer.into());
        report.set_dc_timer(dc_timer.into());
        report.set_power_source_change_debounce(power_policy.into());
        report.set_current_state(current_state.into());
        report.set_vendor_current_state(vendor_current_state);
        report
    }
}

pub(crate) fn serialize_get_alarm_report(
    ac_timer: time_alarm_service_interface::AlarmTimerSeconds,
    dc_timer: time_alarm_service_interface::AlarmTimerSeconds,
    power_policy: time_alarm_service_interface::AlarmExpiredWakePolicy,
    current_state: AlarmCurrentState,
    vendor_current_state: u8,
) -> [u8; 10] {
    GetAlarmReport::new(ac_timer, dc_timer, power_policy, current_state, vendor_current_state).0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub(crate) enum TimeCurrentState {
    Failed = 1,
    Running = 2,
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct GetTimeReport([u8]);
    impl Debug;
    u16, year, set_year: 13, 0;
    u8, month, set_month: 17, 14;
    u8, day, set_day: 22, 18;
    u8, hour, set_hour: 27, 23;
    u8, minute, set_minute: 33, 28;
    u8, second, set_second: 39, 34;
    u16, millisecond, set_millisecond: 49, 40;
    i16, time_zone, set_time_zone: 61, 50;
    dst_observed, set_dst_observed: 62;
    dst_active, set_dst_active: 63;
    u8, current_state, set_current_state: 65, 64;
    u8, vendor_current_state, set_vendor_current_state: 69, 66;
}

impl GetTimeReport<[u8; 9]> {
    fn new(ts: time_alarm_service_interface::AcpiTimestamp, current_state: TimeCurrentState, vendor_state: u8) -> Self {
        let mut report = Self([0; 9]);
        report.set_year(ts.datetime.year());
        report.set_month(ts.datetime.month().into());
        report.set_day(ts.datetime.day());
        report.set_hour(ts.datetime.hour());
        report.set_minute(ts.datetime.minute());
        report.set_second(ts.datetime.second());
        report.set_millisecond((ts.datetime.nanoseconds() / 1_000_000) as u16);
        report.set_time_zone(ts.time_zone.into());
        report.set_dst_observed(matches!(
            ts.dst_status,
            time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotAdjusted
                | time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted
        ));
        report.set_dst_active(matches!(
            ts.dst_status,
            time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted
        ));
        report.set_current_state(current_state.into());
        report.set_vendor_current_state(vendor_state);
        report
    }

    /// Report used when the current time is unavailable; the time fields carry no meaning.
    fn failed(vendor_state: u8) -> Self {
        let mut report = Self([0; 9]);
        report.set_current_state(TimeCurrentState::Failed.into());
        report.set_vendor_current_state(vendor_state);
        report
    }
}

pub(crate) fn serialize_get_time_report(
    timestamp: time_alarm_service_interface::AcpiTimestamp,
    current_state: TimeCurrentState,
    vendor_current_state: u8,
) -> [u8; 9] {
    GetTimeReport::new(timestamp, current_state, vendor_current_state).0
}

pub(crate) fn serialize_failed_get_time_report(vendor_current_state: u8) -> [u8; 9] {
    GetTimeReport::failed(vendor_current_state).0
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

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct SetAlarmReport([u8]);
    impl Debug;
    pub u32, into HidTimerSeconds, timer_seconds, _: 30, 0;
}

impl<'a> SetAlarmReport<&'a [u8; 4]> {
    fn unpack(data: &'a [u8]) -> Result<Self, ReportError> {
        let data = data
            .get(..4)
            .and_then(|data| data.try_into().ok())
            .ok_or(ReportError::InvalidLength)?;
        Ok(Self(data))
    }
}

pub(crate) fn deserialize_set_alarm_report(
    data: &[u8],
) -> Result<time_alarm_service_interface::AlarmTimerSeconds, ReportError> {
    let report = SetAlarmReport::unpack(data)?;
    Ok(report.timer_seconds().into())
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct SetDebounceReport([u8]);
    impl Debug;
    pub u8, into HidWakePolicy, power_source_change_debounce, _: 5, 0;
}

impl<'a> SetDebounceReport<&'a [u8; 1]> {
    fn unpack(data: &'a [u8]) -> Result<Self, ReportError> {
        let data = data.first_chunk::<1>().ok_or(ReportError::InvalidLength)?;
        Ok(Self(data))
    }
}

pub(crate) fn deserialize_set_debounce_report(
    data: &[u8],
) -> Result<time_alarm_service_interface::AlarmExpiredWakePolicy, ReportError> {
    let report = SetDebounceReport::unpack(data)?;
    Ok(report.power_source_change_debounce().into())
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct SetTimeReport([u8]);
    impl Debug;
    pub u16, year, _: 13, 0;
    pub u8, month, _: 17, 14;
    pub u8, day, _: 22, 18;
    pub u8, hour, _: 27, 23;
    pub u8, minute, _: 33, 28;
    pub u8, second, _: 39, 34;
    pub u16, millisecond, _: 49, 40;
    pub i16, time_zone, _: 61, 50;
    pub dst_observed, _: 62;
    pub dst_active, _: 63;
}

impl<'a> SetTimeReport<&'a [u8; 8]> {
    fn unpack(data: &'a [u8]) -> Result<Self, ReportError> {
        let data = data
            .get(..8)
            .and_then(|data| data.try_into().ok())
            .ok_or(ReportError::InvalidLength)?;
        Ok(Self(data))
    }
}

pub(crate) fn deserialize_set_time_report(
    data: &[u8],
) -> Result<time_alarm_service_interface::AcpiTimestamp, ReportError> {
    SetTimeReport::unpack(data)?.try_into()
}

impl<T: AsRef<[u8]>> TryFrom<SetTimeReport<T>> for time_alarm_service_interface::AcpiTimestamp {
    type Error = ReportError;
    fn try_from(report: SetTimeReport<T>) -> Result<Self, Self::Error> {
        Ok(Self {
            datetime: Datetime::new(DatetimeFields {
                year: report.year(),
                month: report.month().try_into().map_err(|_| ReportError::InvalidDatetime)?,
                day: report.day(),
                hour: report.hour(),
                minute: report.minute(),
                second: report.second(),
                nanosecond: u32::from(report.millisecond()) * 1_000_000,
            })?,
            time_zone: AcpiTimeZone::try_from(report.time_zone()).unwrap_or(AcpiTimeZone::Unknown),
            dst_status: match (report.dst_observed(), report.dst_active()) {
                (false, false) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotObserved,
                (true, false) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotAdjusted,
                (true, true) => time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::Adjusted,
                (false, true) => {
                    return Err(ReportError::ValueOutOfRange);
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

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct CapabilitiesFeatureReport([u8]);
    impl Debug;
    pub u8, power_state, set_power_state: 1, 0;
}

impl CapabilitiesFeatureReport<[u8; 1]> {
    fn new(power_state: PowerState) -> Self {
        let mut report = Self([0]);
        report.set_power_state(power_state.into());
        report
    }
}

pub(crate) fn serialize_capabilities_feature_report(power_state: PowerState) -> [u8; 1] {
    CapabilitiesFeatureReport::new(power_state).0
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
    0x16, 0xB2, 0x07,                //     LogicalMinimum(1,970)
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
    0x16, 0xB2, 0x07,                //     LogicalMinimum(1,970)
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn get_alarm_report_matches_hid_bit_layout() {
        let report = serialize_get_alarm_report(
            time_alarm_service_interface::AlarmTimerSeconds(0x1234_5678),
            time_alarm_service_interface::AlarmTimerSeconds(0x2345_6789),
            time_alarm_service_interface::AlarmExpiredWakePolicy(42),
            AlarmCurrentState::Running,
            10,
        );

        assert_eq!(report, [0x78, 0x56, 0x34, 0x12, 0x89, 0x67, 0x45, 0x23, 0x75, 0x0A]);
    }

    #[test]
    fn set_time_report_matches_hid_bit_layout() {
        let data = [0xEA, 0x47, 0x86, 0xD6, 0xEE, 0xE7, 0x83, 0x78];
        let timestamp = deserialize_set_time_report(&data).unwrap();

        assert_eq!(timestamp.datetime.year(), 2026);
        assert_eq!(u8::from(timestamp.datetime.month()), 9);
        assert_eq!(timestamp.datetime.day(), 1);
        assert_eq!(timestamp.datetime.hour(), 13);
        assert_eq!(timestamp.datetime.minute(), 45);
        assert_eq!(timestamp.datetime.second(), 59);
        assert_eq!(timestamp.datetime.nanoseconds(), 999_000_000);
        assert_eq!(
            timestamp.time_zone,
            AcpiTimeZone::MinutesFromUtc(time_alarm_service_interface::AcpiTimeZoneOffset::new(-480).unwrap())
        );
        assert_eq!(
            timestamp.dst_status,
            time_alarm_service_interface::AcpiDaylightSavingsTimeStatus::NotAdjusted
        );
    }

    #[test]
    fn set_reports_deserialize_to_interface_types() {
        assert_eq!(
            deserialize_set_alarm_report(&[42, 0, 0, 0]).unwrap(),
            time_alarm_service_interface::AlarmTimerSeconds(42)
        );
        assert_eq!(
            deserialize_set_alarm_report(&[0, 0, 0, 0]).unwrap(),
            time_alarm_service_interface::AlarmTimerSeconds::DISABLED
        );
        assert_eq!(
            deserialize_set_debounce_report(&[42]).unwrap(),
            time_alarm_service_interface::AlarmExpiredWakePolicy(42)
        );
    }

    #[test]
    fn wire_newtypes_preserve_null_and_clamping_semantics() {
        assert_eq!(
            u32::from(HidTimerSeconds::from(
                time_alarm_service_interface::AlarmTimerSeconds::DISABLED
            )),
            TIMER_NULL
        );
        assert_eq!(
            u32::from(HidTimerSeconds::from(time_alarm_service_interface::AlarmTimerSeconds(
                u32::MAX - 1
            ))),
            TIMER_LOGICAL_MAX
        );
        assert_eq!(
            time_alarm_service_interface::AlarmTimerSeconds::from(HidTimerSeconds::from(TIMER_NULL)),
            time_alarm_service_interface::AlarmTimerSeconds::DISABLED
        );

        assert_eq!(
            u8::from(HidWakePolicy::from(
                time_alarm_service_interface::AlarmExpiredWakePolicy::NEVER
            )),
            DEBOUNCE_LOGICAL_MAX as u8
        );
        assert_eq!(
            time_alarm_service_interface::AlarmExpiredWakePolicy::from(HidWakePolicy::from(0)),
            time_alarm_service_interface::AlarmExpiredWakePolicy::INSTANTLY
        );
    }

    #[test]
    fn unpack_rejects_short_reports() {
        assert_eq!(deserialize_set_alarm_report(&[0; 3]), Err(ReportError::InvalidLength));
        assert_eq!(deserialize_set_debounce_report(&[]), Err(ReportError::InvalidLength));
        assert!(matches!(
            deserialize_set_time_report(&[0; 7]),
            Err(ReportError::InvalidLength)
        ));
    }
}
