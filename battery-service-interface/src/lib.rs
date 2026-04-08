#![no_std]

pub use embedded_batteries_async::acpi::{
    BatteryState, BatterySwapCapability, BatteryTechnology, Bct, BctReturnResult, Bma, Bmc, BmcControlFlags, Bmd,
    BmdCapabilityFlags, BmdStatusFlags, Bms, Bpc, Bps, Bpt, BstReturn, Btm, BtmReturnResult, Btp, PowerSource,
    PowerSourceState, PowerThresholdSupport, PowerUnit, PsrReturn, StaReturn,
};

/// Standard Battery Service Model Number String Size
pub const STD_BIX_MODEL_SIZE: usize = 8;
/// Standard Battery Service Serial Number String Size
pub const STD_BIX_SERIAL_SIZE: usize = 8;
/// Standard Battery Service Battery Type String Size
pub const STD_BIX_BATTERY_SIZE: usize = 8;
/// Standard Battery Service OEM Info String Size
pub const STD_BIX_OEM_SIZE: usize = 8;
/// Standard Power Policy Service Model Number String Size
pub const STD_PIF_MODEL_SIZE: usize = 8;
/// Standard Power Policy Serial Number String Size
pub const STD_PIF_SERIAL_SIZE: usize = 8;
/// Standard Power Policy Service OEM Info String Size
pub const STD_PIF_OEM_SIZE: usize = 8;

#[derive(PartialEq, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BixFixedStrings {
    /// Revision of the BIX structure. Current revision is 1.
    pub revision: u32,
    /// Unit used for capacity and rate values.
    pub power_unit: PowerUnit,
    /// Design capacity of the battery (in mWh or mAh).
    pub design_capacity: u32,
    /// Last full charge capacity (in mWh or mAh).
    pub last_full_charge_capacity: u32,
    /// Battery technology type.
    pub battery_technology: BatteryTechnology,
    /// Design voltage (in mV).
    pub design_voltage: u32,
    /// Warning capacity threshold (in mWh or mAh).
    pub design_cap_of_warning: u32,
    /// Low capacity threshold (in mWh or mAh).
    pub design_cap_of_low: u32,
    /// Number of charge/discharge cycles.
    pub cycle_count: u32,
    /// Measurement accuracy in thousandths of a percent (e.g., 80000 = 80.000%).
    pub measurement_accuracy: u32,
    /// Maximum supported sampling time (in ms).
    pub max_sampling_time: u32,
    /// Minimum supported sampling time (in ms).
    pub min_sampling_time: u32,
    /// Maximum supported averaging interval (in ms).
    pub max_averaging_interval: u32,
    /// Minimum supported averaging interval (in ms).
    pub min_averaging_interval: u32,
    /// Capacity granularity between low and warning (in mWh or mAh).
    pub battery_capacity_granularity_1: u32,
    /// Capacity granularity between warning and full (in mWh or mAh).
    pub battery_capacity_granularity_2: u32,
    /// OEM-specific model number (ASCIIZ).
    pub model_number: [u8; STD_BIX_MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ).
    pub serial_number: [u8; STD_BIX_SERIAL_SIZE],
    /// OEM-specific battery type (ASCIIZ).
    pub battery_type: [u8; STD_BIX_BATTERY_SIZE],
    /// OEM-specific information (ASCIIZ).
    pub oem_info: [u8; STD_BIX_OEM_SIZE],
    /// Battery swapping capability.
    pub battery_swapping_capability: BatterySwapCapability,
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PifFixedStrings {
    /// Bitfield describing the state and characteristics of the power source.
    pub power_source_state: PowerSourceState,
    /// Maximum rated output power in milliwatts (mW).
    ///
    /// 0xFFFFFFFF indicates the value is unavailable.
    pub max_output_power: u32,
    /// Maximum rated input power in milliwatts (mW).
    ///
    /// 0xFFFFFFFF indicates the value is unavailable.
    pub max_input_power: u32,
    /// OEM-specific model number (ASCIIZ). Empty string if not supported.
    pub model_number: [u8; STD_PIF_MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ). Empty string if not supported.
    pub serial_number: [u8; STD_PIF_SERIAL_SIZE],
    /// OEM-specific information (ASCIIZ). Empty string if not supported.
    pub oem_info: [u8; STD_PIF_OEM_SIZE],
}

/// Fuel gauge ID
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

pub trait BatteryService {
    /// Queries the estimated time remaining until the battery reaches the specified charge level. Corresponds to ACPI's _BCT method
    fn battery_charge_time(
        &self,
        battery_id: DeviceId,
        charge_level: Bct,
    ) -> impl core::future::Future<Output = Result<BctReturnResult, BatteryError>>;

    /// Returns static information about the battery. Corresponds to ACPI's _BIX method.
    fn battery_info(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<BixFixedStrings, BatteryError>>;

    /// Sets the averaging interval of battery capacity measurement in milliseconds. Corresponds to ACPI's _BMA method.
    fn set_battery_measurement_averaging_interval(
        &self,
        battery_id: DeviceId,
        bma: Bma,
    ) -> impl core::future::Future<Output = Result<(), BatteryError>>;

    /// Battery maintenance control. Corresponds to ACPI's _BMC method.
    fn battery_maintenance_control(
        &self,
        battery_id: DeviceId,
        bmc: Bmc,
    ) -> impl core::future::Future<Output = Result<(), BatteryError>>;

    /// Retrieves battery maintenance data. Corresponds to ACPI's _BMD method.
    fn battery_maintenance_data(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<Bmd, BatteryError>>;

    /// Sets the battery measurement sampling time in milliseconds. Corresponds to ACPI's _BMS method.
    fn set_battery_measurement_sampling_time(
        &self,
        battery_id: DeviceId,
        battery_measurement_sampling: Bms,
    ) -> impl core::future::Future<Output = Result<(), BatteryError>>;

    /// Queries the current power characteristics of the battery. Corresponds to ACPI's _BPC method.
    fn battery_power_characteristics(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<Bpc, BatteryError>>;

    /// Queries the current state of the battery. Corresponds to ACPI's _BPS method.
    fn battery_power_state(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<Bps, BatteryError>>;

    /// Sets battery power threshold. Corresponds to ACPI's _BPT method.
    fn set_battery_power_threshold(
        &self,
        battery_id: DeviceId,
        power_threshold: Bpt,
    ) -> impl core::future::Future<Output = Result<(), BatteryError>>;

    /// Queries the battery's current estimated remaining capacity. Corresponds to ACPI's _BST method.
    fn battery_status(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<BstReturn, BatteryError>>;

    /// Queries the estimated time remaining until the battery is fully discharged at the current discharge rate. Corresponds to ACPI's _BTM method.
    fn battery_time_to_empty(
        &self,
        battery_id: DeviceId,
        battery_discharge_rate: Btm,
    ) -> impl core::future::Future<Output = Result<BtmReturnResult, BatteryError>>;

    /// Sets a battery trip point. Corresponds to ACPI's _BTP method.
    fn set_battery_trip_point(
        &self,
        battery_id: DeviceId,
        btp: Btp,
    ) -> impl core::future::Future<Output = Result<(), BatteryError>>;

    /// Queries whether the battery is currently in use (i.e., providing power to the system). Corresponds to ACPI's _PSR method.
    fn is_in_use(&self, battery_id: DeviceId) -> impl core::future::Future<Output = Result<PsrReturn, BatteryError>>;

    /// Queries information about the battery's power source. Corresponds to ACPI's _PIF method.
    fn power_source_information(
        &self,
        power_source_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<PifFixedStrings, BatteryError>>;

    /// Queries the battery's status. Corresponds to ACPI's _STA method.
    fn device_status(
        &self,
        battery_id: DeviceId,
    ) -> impl core::future::Future<Output = Result<StaReturn, BatteryError>>;
}

#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Errors that can occur when interacting with the battery service.
pub enum BatteryError {
    /// The specified battery ID does not correspond to any known battery.
    UnknownDeviceId,

    /// An unknown error occurred while processing the request.
    UnspecifiedFailure,
}
