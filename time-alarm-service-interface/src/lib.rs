#![no_std]

mod acpi_timestamp;
pub use acpi_timestamp::{AcpiDaylightSavingsTimeStatus, AcpiTimeZone, AcpiTimeZoneOffset, AcpiTimestamp};

use bitfield::bitfield;
use embedded_mcu_hal::time::DatetimeClockError;

/// The number of seconds before a timer will expire.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlarmTimerSeconds(pub u32);
impl AlarmTimerSeconds {
    /// The alarm is not running and will never expire.
    pub const DISABLED: Self = Self(u32::MAX);
}

impl Default for AlarmTimerSeconds {
    fn default() -> Self {
        Self::DISABLED
    }
}

// -------------------------------------------------

/// If a timer is on the wrong power source when it expires, the number of seconds after switching to the correct
/// power source that must elapse on the correct power source before the timer actually triggers a wake event.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlarmExpiredWakePolicy(pub u32);
impl AlarmExpiredWakePolicy {
    /// The timer will trigger a wake event immediately upon switching to the correct power source.
    pub const INSTANTLY: Self = Self(0);

    /// The timer will never trigger a wake event if it expires on the wrong power source, even if it later switches to the correct power source.
    pub const NEVER: Self = Self(u32::MAX);
}

impl Default for AlarmExpiredWakePolicy {
    fn default() -> Self {
        Self::NEVER
    }
}

// -------------------------------------------------

/// ACPI timer ID as defined in the ACPI spec.
#[derive(Clone, Copy, Debug, PartialEq, num_enum::TryFromPrimitive, num_enum::IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
pub enum AcpiTimerId {
    /// The timer that is active when the system is on external power.
    AcPower = 0,

    /// The timer that is active when the system is on battery power.
    DcPower = 1,
}

impl AcpiTimerId {
    /// Gets the timer ID for the other power source.
    pub fn get_other_timer_id(&self) -> Self {
        match self {
            AcpiTimerId::AcPower => AcpiTimerId::DcPower,
            AcpiTimerId::DcPower => AcpiTimerId::AcPower,
        }
    }
}

bitfield!(
    /// Describes the current status of a timer, including whether it has expired and whether it triggered a wake event.
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
    /// Describes the capabilities of a time-alarm device. Details on semantics of individual fields are available in the ACPI spec, version 6.4, section 9.18.2
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

/// The interface for a time-alarm service, which implements the ACPI Time and Alarm device specification.
/// See the ACPI spec version 6.4, section 9.18, for more details on the expected behavior of each method.
pub trait TimeAlarmService {
    /// Query the capabilities of the time-alarm device.  Analogous to ACPI TAD's _GCP method.
    fn get_capabilities(&self) -> TimeAlarmDeviceCapabilities;

    /// Query the current time.  Analogous to ACPI TAD's _GRT method.
    fn get_real_time(&self) -> Result<AcpiTimestamp, DatetimeClockError>;

    /// Change the current time.  Analogous to ACPI TAD's _SRT method.
    fn set_real_time(&self, timestamp: AcpiTimestamp) -> Result<(), DatetimeClockError>;

    /// Query the current wake status.  Analogous to ACPI TAD's _GWS method.
    fn get_wake_status(&self, timer_id: AcpiTimerId) -> TimerStatus;

    /// Clear the current wake status.  Analogous to ACPI TAD's _CWS method.
    fn clear_wake_status(&self, timer_id: AcpiTimerId);

    /// Configures behavior when the timer expires while the system is on the other power source.  Analogous to ACPI TAD's _STP method.
    fn set_expired_timer_policy(
        &self,
        timer_id: AcpiTimerId,
        policy: AlarmExpiredWakePolicy,
    ) -> Result<(), DatetimeClockError>;

    /// Query current behavior when the timer expires while the system is on the other power source.  Analogous to ACPI TAD's _TIP method.
    fn get_expired_timer_policy(&self, timer_id: AcpiTimerId) -> AlarmExpiredWakePolicy;

    /// Change the expiry time for the given timer.  Analogous to ACPI TAD's _STV method.
    fn set_timer_value(&self, timer_id: AcpiTimerId, timer_value: AlarmTimerSeconds) -> Result<(), DatetimeClockError>;

    /// Query the expiry time for the given timer.  Analogous to ACPI TAD's _TIV method.
    fn get_timer_value(&self, timer_id: AcpiTimerId) -> Result<AlarmTimerSeconds, DatetimeClockError>;
}
