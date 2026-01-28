#![no_std]

mod acpi_timestamp;
pub use acpi_timestamp::{AcpiDaylightSavingsTimeStatus, AcpiTimeZone, AcpiTimeZoneOffset, AcpiTimestamp};

use bitfield::bitfield;
use core::array::TryFromSliceError;
use embedded_services::relay::{MessageSerializationError, SerializableMessage};

/// Message types for the ACPI Time and Alarm device service.
/// These are directly analogous to the ACPI Time and Alarm device methods.
/// See ACPI Specification 6.4, Section 9.18 "Time and Alarm Device" for additional details on semantics.
#[rustfmt::skip]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AcpiTimeAlarmRequest {
    GetCapabilities,                                            // _GCP
    GetRealTime,                                                // _GRT
    SetRealTime(AcpiTimestamp),                                 // _SRT
    GetWakeStatus(AcpiTimerId),                                 // _GWS
    ClearWakeStatus(AcpiTimerId),                               // _CWS
    SetTimerValue(AcpiTimerId, AlarmTimerSeconds),              // _STV
    GetTimerValue(AcpiTimerId),                                 // _TIV
    SetExpiredTimerPolicy(AcpiTimerId, AlarmExpiredWakePolicy), // _STP
    GetExpiredTimerPolicy(AcpiTimerId),                         // _TIP
}

#[derive(Clone, Copy, Debug, PartialEq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[repr(u16)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum AcpiTimeAlarmRequestDiscriminant {
    GetCapabilities = 1,
    GetRealTime = 2,
    SetRealTime = 3,
    GetWakeStatus = 4,
    ClearWakeStatus = 5,
    SetTimerValue = 6,
    GetTimerValue = 7,
    SetExpiredTimerPolicy = 8,
    GetExpiredTimerPolicy = 9,
}

impl SerializableMessage for AcpiTimeAlarmRequest {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::GetCapabilities => Ok(0),
            Self::GetRealTime => Ok(0),
            Self::SetRealTime(timestamp) => {
                let serialized = timestamp.as_bytes();
                buffer
                    .split_at_mut_checked(serialized.len())
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .0
                    .copy_from_slice(&serialized);
                Ok(serialized.len())
            }
            Self::GetWakeStatus(timer_id)
            | Self::ClearWakeStatus(timer_id)
            | Self::GetTimerValue(timer_id)
            | Self::GetExpiredTimerPolicy(timer_id) => safe_put_u32(buffer, 0, timer_id.into()),

            Self::SetTimerValue(timer_id, alarm_timer_seconds) => {
                safe_put_u32(buffer, 0, timer_id.into())?;
                safe_put_u32(buffer, 4, alarm_timer_seconds.0)?;
                Ok(8)
            }
            Self::SetExpiredTimerPolicy(timer_id, alarm_expired_wake_policy) => {
                safe_put_u32(buffer, 0, timer_id.into())?;
                safe_put_u32(buffer, 4, alarm_expired_wake_policy.0)?;
                Ok(8)
            }
        }
    }

    fn discriminant(&self) -> u16 {
        match self {
            AcpiTimeAlarmRequest::GetCapabilities => AcpiTimeAlarmRequestDiscriminant::GetCapabilities.into(),
            AcpiTimeAlarmRequest::GetRealTime => AcpiTimeAlarmRequestDiscriminant::GetRealTime.into(),
            AcpiTimeAlarmRequest::SetRealTime(_) => AcpiTimeAlarmRequestDiscriminant::SetRealTime.into(),
            AcpiTimeAlarmRequest::GetWakeStatus(_) => AcpiTimeAlarmRequestDiscriminant::GetWakeStatus.into(),
            AcpiTimeAlarmRequest::ClearWakeStatus(_) => AcpiTimeAlarmRequestDiscriminant::ClearWakeStatus.into(),
            AcpiTimeAlarmRequest::SetTimerValue(_, _) => AcpiTimeAlarmRequestDiscriminant::SetTimerValue.into(),
            AcpiTimeAlarmRequest::GetTimerValue(_) => AcpiTimeAlarmRequestDiscriminant::GetTimerValue.into(),
            AcpiTimeAlarmRequest::SetExpiredTimerPolicy(_, _) => {
                AcpiTimeAlarmRequestDiscriminant::SetExpiredTimerPolicy.into()
            }
            AcpiTimeAlarmRequest::GetExpiredTimerPolicy(_) => {
                AcpiTimeAlarmRequestDiscriminant::GetExpiredTimerPolicy.into()
            }
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        let discriminant = AcpiTimeAlarmRequestDiscriminant::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?;
        match discriminant {
            AcpiTimeAlarmRequestDiscriminant::GetCapabilities => Ok(AcpiTimeAlarmRequest::GetCapabilities),
            AcpiTimeAlarmRequestDiscriminant::GetRealTime => Ok(AcpiTimeAlarmRequest::GetRealTime),
            AcpiTimeAlarmRequestDiscriminant::SetRealTime => Ok(AcpiTimeAlarmRequest::SetRealTime(
                AcpiTimestamp::try_from_bytes(buffer)
                    .map_err(|_| MessageSerializationError::InvalidPayload("Could not deserialize timestamp"))?,
            )),
            _ => {
                let (timer_id, buffer) = buffer
                    .split_at_checked(4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?;
                let timer_id = AcpiTimerId::try_from(u32::from_le_bytes(
                    timer_id
                        .try_into()
                        .map_err(|_| MessageSerializationError::BufferTooSmall)?,
                ))
                .map_err(|_| MessageSerializationError::InvalidPayload("Could not deserialize timer ID"))?;

                match discriminant {
                    AcpiTimeAlarmRequestDiscriminant::GetWakeStatus => {
                        Ok(AcpiTimeAlarmRequest::GetWakeStatus(timer_id))
                    }
                    AcpiTimeAlarmRequestDiscriminant::ClearWakeStatus => {
                        Ok(AcpiTimeAlarmRequest::ClearWakeStatus(timer_id))
                    }
                    AcpiTimeAlarmRequestDiscriminant::SetTimerValue => Ok(AcpiTimeAlarmRequest::SetTimerValue(
                        timer_id,
                        AlarmTimerSeconds(u32::from_le_bytes(
                            buffer
                                .try_into()
                                .map_err(|_| MessageSerializationError::BufferTooSmall)?,
                        )),
                    )),
                    AcpiTimeAlarmRequestDiscriminant::GetTimerValue => {
                        Ok(AcpiTimeAlarmRequest::GetTimerValue(timer_id))
                    }
                    AcpiTimeAlarmRequestDiscriminant::SetExpiredTimerPolicy => {
                        Ok(AcpiTimeAlarmRequest::SetExpiredTimerPolicy(
                            timer_id,
                            AlarmExpiredWakePolicy(u32::from_le_bytes(
                                buffer
                                    .try_into()
                                    .map_err(|_| MessageSerializationError::BufferTooSmall)?,
                            )),
                        ))
                    }
                    AcpiTimeAlarmRequestDiscriminant::GetExpiredTimerPolicy => {
                        Ok(AcpiTimeAlarmRequest::GetExpiredTimerPolicy(timer_id))
                    }
                    _ => Err(MessageSerializationError::UnknownMessageDiscriminant(
                        discriminant.into(),
                    )),
                }
            }
        }
    }
}

// -------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlarmTimerSeconds(pub u32);
impl AlarmTimerSeconds {
    pub const DISABLED: Self = Self(u32::MAX);
}

impl Default for AlarmTimerSeconds {
    fn default() -> Self {
        Self::DISABLED
    }
}

// -------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlarmExpiredWakePolicy(pub u32);
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

// Timer ID as defined in the ACPI spec.
#[derive(Clone, Copy, Debug, PartialEq, num_enum::TryFromPrimitive, num_enum::IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
pub enum AcpiTimerId {
    AcPower = 0,
    DcPower = 1,
}

impl AcpiTimerId {
    pub fn get_other_timer_id(&self) -> Self {
        match self {
            AcpiTimerId::AcPower => AcpiTimerId::DcPower,
            AcpiTimerId::DcPower => AcpiTimerId::AcPower,
        }
    }
}

bitfield!(
    #[derive(Copy, Clone, Default, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct TimerStatus(u32);
    impl Debug;
    bool;
    pub timer_expired, set_timer_expired: 0;
    pub timer_triggered_wake, set_timer_triggered_wake: 1;
);

// -------------------------------------------------

bitfield!(
    #[derive(Copy, Clone, Default, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct TimeAlarmDeviceCapabilities(u32);
    impl Debug;
    bool;
    pub ac_wake_implemented, set_ac_wake_implemented: 0;
    pub dc_wake_implemented, set_dc_wake_implemented: 1;
    pub realtime_implemented, set_realtime_implemented: 2;
    pub realtime_accuracy_in_milliseconds, set_realtime_accuracy_in_milliseconds: 3;
    pub get_wake_status_supported, set_get_wake_status_supported: 4;
    pub ac_s4_wake_supported, set_ac_s4_wake_supported: 5;
    pub ac_s5_wake_supported, set_ac_s5_wake_supported: 6;
    pub dc_s4_wake_supported, set_dc_s4_wake_supported: 7;
    pub dc_s5_wake_supported, set_dc_s5_wake_supported: 8;
);

// -------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AcpiTimeAlarmResponse {
    Capabilities(TimeAlarmDeviceCapabilities),
    RealTime(AcpiTimestamp),
    TimerStatus(TimerStatus),
    WakePolicy(AlarmExpiredWakePolicy),
    TimerSeconds(AlarmTimerSeconds),

    /// Operation succeeded, but there's no data to return.
    OkNoData,
}

#[derive(Copy, Clone, Debug, PartialEq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[repr(u16)]
enum AcpiTimeAlarmResponseDiscriminant {
    Capabilities = 1,
    RealTime = 2,
    TimerStatus = 3,
    WakePolicy = 4,
    TimerSeconds = 5,
    OkNoData = 6,
}

impl SerializableMessage for AcpiTimeAlarmResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::Capabilities(capabilities) => safe_put_u32(buffer, 0, capabilities.0),
            Self::RealTime(timestamp) => {
                let result = timestamp.as_bytes();
                buffer
                    .split_at_mut_checked(result.len())
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .0
                    .copy_from_slice(&result);
                Ok(result.len())
            }
            Self::TimerStatus(timer_status) => safe_put_u32(buffer, 0, timer_status.0),
            Self::WakePolicy(wake_policy) => safe_put_u32(buffer, 0, wake_policy.0),
            Self::TimerSeconds(timer_seconds) => safe_put_u32(buffer, 0, timer_seconds.0),
            Self::OkNoData => Ok(0),
        }
    }

    fn discriminant(&self) -> u16 {
        match self {
            Self::Capabilities(_) => AcpiTimeAlarmResponseDiscriminant::Capabilities.into(),
            Self::RealTime(_) => AcpiTimeAlarmResponseDiscriminant::RealTime.into(),
            Self::TimerStatus(_) => AcpiTimeAlarmResponseDiscriminant::TimerStatus.into(),
            Self::WakePolicy(_) => AcpiTimeAlarmResponseDiscriminant::WakePolicy.into(),
            Self::TimerSeconds(_) => AcpiTimeAlarmResponseDiscriminant::TimerSeconds.into(),
            Self::OkNoData => AcpiTimeAlarmResponseDiscriminant::OkNoData.into(),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        let discriminant = AcpiTimeAlarmResponseDiscriminant::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?;
        match discriminant {
            AcpiTimeAlarmResponseDiscriminant::Capabilities => Ok(Self::Capabilities(TimeAlarmDeviceCapabilities(
                safe_get_u32(buffer, 0)?,
            ))),
            AcpiTimeAlarmResponseDiscriminant::RealTime => {
                Ok(Self::RealTime(AcpiTimestamp::try_from_bytes(buffer).map_err(|_| {
                    MessageSerializationError::InvalidPayload("invalid timestamp")
                })?))
            }
            AcpiTimeAlarmResponseDiscriminant::TimerStatus => {
                Ok(Self::TimerStatus(TimerStatus(safe_get_u32(buffer, 0)?)))
            }
            AcpiTimeAlarmResponseDiscriminant::WakePolicy => {
                Ok(Self::WakePolicy(AlarmExpiredWakePolicy(safe_get_u32(buffer, 0)?)))
            }
            AcpiTimeAlarmResponseDiscriminant::TimerSeconds => {
                Ok(Self::TimerSeconds(AlarmTimerSeconds(safe_get_u32(buffer, 0)?)))
            }
            AcpiTimeAlarmResponseDiscriminant::OkNoData => Ok(Self::OkNoData),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum AcpiTimeAlarmError {
    UnspecifiedFailure = 1,
}

impl SerializableMessage for AcpiTimeAlarmError {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::UnspecifiedFailure => Ok(0),
        }
    }

    fn discriminant(&self) -> u16 {
        (*self).into()
    }

    fn deserialize(discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        let discriminant = AcpiTimeAlarmError::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?;

        match discriminant {
            AcpiTimeAlarmError::UnspecifiedFailure => Ok(AcpiTimeAlarmError::UnspecifiedFailure),
        }
    }
}

impl From<embedded_mcu_hal::time::DatetimeError> for AcpiTimeAlarmError {
    fn from(_error: embedded_mcu_hal::time::DatetimeError) -> Self {
        AcpiTimeAlarmError::UnspecifiedFailure
    }
}

impl From<num_enum::TryFromPrimitiveError<AcpiDaylightSavingsTimeStatus>> for AcpiTimeAlarmError {
    fn from(_error: num_enum::TryFromPrimitiveError<AcpiDaylightSavingsTimeStatus>) -> Self {
        AcpiTimeAlarmError::UnspecifiedFailure
    }
}

impl From<TryFromSliceError> for AcpiTimeAlarmError {
    fn from(_error: TryFromSliceError) -> Self {
        AcpiTimeAlarmError::UnspecifiedFailure
    }
}

pub type AcpiTimeAlarmResult = Result<AcpiTimeAlarmResponse, AcpiTimeAlarmError>;

fn safe_put_u32(buffer: &mut [u8], index: usize, val: u32) -> Result<usize, MessageSerializationError> {
    let val = val.to_le_bytes();
    buffer
        .get_mut(index..index + val.len())
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .copy_from_slice(&val);
    Ok(val.len())
}

fn safe_get_u32(buffer: &[u8], index: usize) -> Result<u32, MessageSerializationError> {
    let bytes = buffer
        .get(index..index + 4)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)?;
    Ok(u32::from_le_bytes(bytes))
}
