use embedded_mcu_hal::time::{Datetime, UncheckedDatetime};

use crate::TimeAlarmError;

// Timestamp structure as specified in the ACPI spec.  Must be exactly this layout.
// TODO [TESTING] are there any endianness shenanigans associated with bytemuck here?
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Copy, Clone, Debug)]
struct RawAcpiTimestamp {
    // Year: 1900 - 9999
    year: u16,

    // Month: 1 - 12
    month: u8,

    // Day: 1 - 31
    day: u8,

    // Hour: 0 - 23
    hour: u8,

    // Minute: 0 - 59
    minute: u8,

    // Second: 0 - 59. Leap seconds are not supported.
    second: u8,

    // For _GRT, 0 = time is not valid (request failed), 1 = time is valid.  For _SRT, this is padding and should be 0.
    valid_or_padding: u8,

    // Millseconds: 1-1000. Leap seconds are not supported.
    // TODO [SPEC] The ACPI spec says 1-1000, but it seems like it should be 0-999? We may need to do some translation if this isn't just a typo in the spec.
    milliseconds: u16,

    // Time zone: -1440 to 1440 in minutes from UTC, or 2047 if unspecified
    time_zone: i16,

    // 1 = daylight savings time in effect, 0 = standard time
    daylight: u8,

    // Reserved, must be 0
    _padding: [u8; 3]
}

impl RawAcpiTimestamp {
    // Try to interpret a byte slice as an AcpiTimestamp.  The slice must be exactly 16 bytes long.
    // Validity of the fields is not checked here.
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, TimeAlarmError> {
        bytemuck::try_pod_read_unaligned(bytes).map_err(|_| TimeAlarmError::InvalidArgument)
    }

    // Get a byte slice representing this AcpiTimestamp.
    pub fn as_bytes(&self) -> &[u8; core::mem::size_of::<Self>()] /* 16 */ {
        bytemuck::bytes_of(self).try_into().expect("Should never fail because we know the size of AcpiTimestamp at compile time")
    }
}

impl From<&AcpiTimestamp> for RawAcpiTimestamp {
    fn from(ts: &AcpiTimestamp) -> Self {
        Self {
            year: ts.datetime.year(),
            month: ts.datetime.month(),
            day: ts.datetime.day(),
            hour: ts.datetime.hour(),
            minute: ts.datetime.minute(),
            second: ts.datetime.second(),
            valid_or_padding: 1, // valid
            milliseconds: (ts.datetime.nanoseconds() / 1_000_000).try_into().expect("Datetime::nanoseconds() is capped at 10^9 and therefore should always divide by 10^6 into something that fits in u16"),
            time_zone: ts.time_zone.into(),
            daylight: ts.dst_status.into(),
            _padding: [0; 3],
        }
    }
}

// -------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub enum AcpiDaylightSavingsTimeStatus {
    /// Daylight savings time is not observed in this timezone.
    NotObserved,

    /// Daylight savings time is observed in this timezone, but the current time has not been adjusted for it.
    NotAdjusted,

    /// Daylight savings time is observed in this timezone, and the current time has been adjusted for it.
    Adjusted
}

impl TryFrom<u8> for AcpiDaylightSavingsTimeStatus {
    type Error = TimeAlarmError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NotObserved),
            1 => Ok(Self::NotAdjusted),
            3 => Ok(Self::Adjusted),
            _ => Err(TimeAlarmError::InvalidArgument)
        }
    }
}

impl From<AcpiDaylightSavingsTimeStatus> for u8 {
    fn from(val: AcpiDaylightSavingsTimeStatus) -> Self {
        match val {
            AcpiDaylightSavingsTimeStatus::NotObserved => 0,
            AcpiDaylightSavingsTimeStatus::NotAdjusted => 1,
            AcpiDaylightSavingsTimeStatus::Adjusted => 3,
        }
    }
}

// -------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct AcpiTimeZoneOffset {
    minutes_from_utc: i16 // minutes from UTC
}

impl AcpiTimeZoneOffset {
    pub fn new(minutes_from_utc: i16) -> Result<Self, TimeAlarmError> {
        if !(-1440..=1440).contains(&minutes_from_utc) {
            return Err(TimeAlarmError::InvalidArgument);
        }
        Ok(Self {
            minutes_from_utc
        })
    }

    pub fn minutes_from_utc(&self) -> i16 {
        self.minutes_from_utc
    }
}

#[derive(Copy, Clone, Debug)]
pub enum AcpiTimeZone {
    /// The time zone is not specified and no relation to UTC can be inferred.
    Unknown,

    /// The time zone is this many minutes from UTC.
    MinutesFromUtc(AcpiTimeZoneOffset)
}

impl TryFrom<i16> for AcpiTimeZone {
    type Error = TimeAlarmError;

    fn try_from(value: i16) -> Result<Self, TimeAlarmError> {
        if value == 2047 {
            Ok(Self::Unknown)
        } else {
            Ok(Self::MinutesFromUtc(AcpiTimeZoneOffset::new(value)?))
        }
    }
}

impl From<AcpiTimeZone> for i16 {
    fn from(val: AcpiTimeZone) -> Self {
        match val {
            AcpiTimeZone::Unknown => 2047,
            AcpiTimeZone::MinutesFromUtc(offset) => offset.minutes_from_utc()
        }
    }
}

// -------------------------------------------------

pub(crate) struct AcpiTimestamp {
    pub datetime: Datetime,
    pub time_zone: AcpiTimeZone,
    pub dst_status: AcpiDaylightSavingsTimeStatus
}

impl AcpiTimestamp {
    pub fn as_bytes(&self) -> [u8; core::mem::size_of::<RawAcpiTimestamp>()] /* 16 */ {
        *RawAcpiTimestamp::from(self).as_bytes()
    }

    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, TimeAlarmError> {
        let raw = RawAcpiTimestamp::try_from_bytes(bytes)?;

        Ok(Self {
            datetime: Datetime::new(UncheckedDatetime {
                year: raw.year,
                month: raw.month,
                day: raw.day,
                hour: raw.hour,
                minute: raw.minute,
                second: raw.second,
                nanosecond: (raw.milliseconds as u32) * 1_000_000,
            })?,
            time_zone: raw.time_zone.try_into()?,
            dst_status: raw.daylight.try_into()?,
        })
    }
}
