use embedded_mcu_hal::time::{Datetime, Month, UncheckedDatetime};

use crate::AcpiTimeAlarmError;
use zerocopy::{FromBytes, I16, Immutable, IntoBytes, KnownLayout, LE, U16, Unaligned};

// Timestamp structure as specified in the ACPI spec.  Must be exactly this layout.
#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Copy, Clone, Debug)]
struct RawAcpiTimestamp {
    // Year: 1900 - 9999
    year: U16<LE>,

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

    // Millseconds: 0-999. Leap seconds are not supported.
    milliseconds: U16<LE>,

    // Time zone: -1440 to 1440 in minutes from UTC, or 2047 if unspecified
    time_zone: I16<LE>,

    // 1 = daylight savings time in effect, 0 = standard time
    daylight: u8,

    // Reserved, must be 0
    _padding: [u8; 3],
}

impl From<&AcpiTimestamp> for RawAcpiTimestamp {
    fn from(ts: &AcpiTimestamp) -> Self {
        Self {
            year: ts.datetime.year().into(),
            month: ts.datetime.month().into(),
            day: ts.datetime.day(),
            hour: ts.datetime.hour(),
            minute: ts.datetime.minute(),
            second: ts.datetime.second(),
            valid_or_padding: 1, // valid
            milliseconds: (ts.datetime.nanoseconds() / 1_000_000).try_into().expect("Datetime::nanoseconds() is capped at 10^9 and therefore should always divide by 10^6 into something that fits in u16"),
            time_zone: i16::from(ts.time_zone).into(),
            daylight: ts.dst_status.into(),
            _padding: [0; 3],
        }
    }
}

// -------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum AcpiDaylightSavingsTimeStatus {
    /// Daylight savings time is not observed in this timezone.
    NotObserved = 0,

    /// Daylight savings time is observed in this timezone, but the current time has not been adjusted for it.
    NotAdjusted = 1,

    // Note: in the spec, this is a pair of flags where bit 0 = observed, bit 1 = adjusted.  2 (adjusted but not observed) is nonsensical, so we omit it.
    //
    /// Daylight savings time is observed in this timezone, and the current time has been adjusted for it.
    Adjusted = 3,
}

// -------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AcpiTimeZoneOffset {
    minutes_from_utc: i16, // minutes from UTC
}

impl AcpiTimeZoneOffset {
    pub fn new(minutes_from_utc: i16) -> Result<Self, AcpiTimeAlarmError> {
        if !(-1440..=1440).contains(&minutes_from_utc) {
            Err(AcpiTimeAlarmError::UnspecifiedFailure)
        } else {
            Ok(Self { minutes_from_utc })
        }
    }

    pub fn minutes_from_utc(&self) -> i16 {
        self.minutes_from_utc
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AcpiTimeZone {
    /// The time zone is not specified and no relation to UTC can be inferred.
    Unknown,

    /// The time zone is this many minutes from UTC.
    MinutesFromUtc(AcpiTimeZoneOffset),
}

impl TryFrom<i16> for AcpiTimeZone {
    type Error = AcpiTimeAlarmError;

    fn try_from(value: i16) -> Result<Self, AcpiTimeAlarmError> {
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
            AcpiTimeZone::MinutesFromUtc(offset) => offset.minutes_from_utc(),
        }
    }
}

// -------------------------------------------------

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(PartialEq, Clone, Copy)]
pub struct AcpiTimestamp {
    pub datetime: Datetime,
    pub time_zone: AcpiTimeZone,
    pub dst_status: AcpiDaylightSavingsTimeStatus,
}

impl AcpiTimestamp {
    pub fn as_bytes(&self) -> [u8; core::mem::size_of::<RawAcpiTimestamp>()] /* 16 */ {
        RawAcpiTimestamp::from(self)
            .as_bytes()
            .try_into()
            .expect("Size is guaranteed to be the size of RawAcpiTimestamp")
    }

    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, AcpiTimeAlarmError> {
        let raw = RawAcpiTimestamp::ref_from_bytes(
            bytes
                .get(..core::mem::size_of::<RawAcpiTimestamp>())
                .ok_or(AcpiTimeAlarmError::UnspecifiedFailure)?,
        )
        .map_err(|_| AcpiTimeAlarmError::UnspecifiedFailure)?;

        Ok(Self {
            datetime: Datetime::new(UncheckedDatetime {
                year: raw.year.get(),
                month: Month::try_from(raw.month).map_err(|_| AcpiTimeAlarmError::UnspecifiedFailure)?,
                day: raw.day,
                hour: raw.hour,
                minute: raw.minute,
                second: raw.second,
                nanosecond: (raw.milliseconds.get() as u32) * 1_000_000,
            })?,
            time_zone: raw.time_zone.get().try_into()?,
            dst_status: raw.daylight.try_into()?,
        })
    }
}
