//! Fuel gauge trait, state machine, and cached battery data.
//!
//! Device drivers implement the [`FuelGauge`] trait (which extends
//! [`embedded_batteries_async::smart_battery::SmartBattery`]) and own their own
//! [`State`]. The battery service drives the fuel gauge entirely through direct
//! async calls on the trait. State transitions are driven by
//! the driver (trait implementer) by calling the `on_*` methods on [`State`]
//! (accessed via [`FuelGauge::state_mut`]).

use core::future::Future;

use embedded_batteries_async::{
    acpi::{BmcControlFlags, BmdCapabilityFlags, BmdStatusFlags, PowerThresholdSupport},
    smart_battery::BatteryModeFields,
};

/// Fuel gauge errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum FuelGaugeError {
    /// The fuel gauge hardware timed out responding.
    Timeout,
    /// The underlying bus reported an error.
    BusError,
}

impl From<core::convert::Infallible> for FuelGaugeError {
    fn from(_value: core::convert::Infallible) -> Self {
        Self::BusError
    }
}

/// Standard static battery data cache.
#[derive(Default, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StaticBatteryMsgs {
    /// Manufacturer Name.
    pub manufacturer_name: [u8; 21],

    /// Device Name.
    pub device_name: [u8; 21],

    /// Device Chemistry.
    pub device_chemistry: [u8; 5],

    /// Design Capacity in mWh.
    pub design_capacity_mwh: u32,

    /// Design Voltage in mV.
    pub design_voltage_mv: u16,

    /// Device Chemistry Id.
    pub device_chemistry_id: [u8; 2],

    /// Device Serial Number.
    pub serial_num: [u8; 4],

    /// Battery Mode.
    pub battery_mode: BatteryModeFields,

    pub design_cap_warning: u32,

    pub design_cap_low: u32,

    pub measurement_accuracy: u32,

    pub max_sample_time: u32,

    pub min_sample_time: u32,

    pub max_averaging_interval: u32,

    pub min_averaging_interval: u32,

    pub cap_granularity_1: u32,

    pub cap_granularity_2: u32,

    pub power_threshold_support: PowerThresholdSupport,

    pub max_instant_pwr_threshold: u32,

    pub max_sus_pwr_threshold: u32,

    pub bmc_flags: BmcControlFlags,

    pub bmd_capability: BmdCapabilityFlags,

    pub bmd_recalibrate_count: u32,

    pub bmd_quick_recalibrate_time: u32,

    pub bmd_slow_recalibrate_time: u32,
}

/// Standard dynamic battery data cache.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DynamicBatteryMsgs {
    /// Battery Max Power in mW.
    pub max_power_mw: u32,

    /// Battery Sustained Power in mW.
    pub sus_power_mw: u32,

    /// Turbo Load Voltage in mV.
    pub turbo_vload_mv: u32,

    /// Turbo RHF Effective in mOhm.
    pub turbo_rhf_effective_mohm: u32,

    /// Full Charge Capacity in mWh.
    pub full_charge_capacity_mwh: u32,

    /// Remaining Capacity in mWh.
    pub remaining_capacity_mwh: u32,

    /// Rsoc in %.
    pub relative_soc_pct: u16,

    /// Charge/Discharge Cycle Count.
    pub cycle_count: u16,

    /// Battery Voltage in mV.
    pub voltage_mv: u16,

    /// Maximum Error in %.
    pub max_error_pct: u16,

    /// Battery Status (Standard Smart Battery Defined).
    pub battery_status: u16,

    /// Desired Charging Voltage in mV.
    pub charging_voltage_mv: u16,

    /// Desired Charging Current in mA.
    pub charging_current_ma: u16,

    /// Battery Temperature in dK.
    pub battery_temp_dk: u16,

    /// Battery Current in mA.
    pub current_ma: i16,

    /// Battery Avg Current.
    pub average_current_ma: i16,

    pub bmd_status: BmdStatusFlags,
}

/// Operational state substates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OperationalSubstate {
    /// The fuel gauge is initialized but has not yet collected static data.
    Init,
    /// The fuel gauge is initialized and ready to be polled for dynamic data.
    Polling,
}

/// Present state substates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PresentSubstate {
    /// The fuel gauge is present but communication has been lost and recovery is required.
    NotOperational,
    /// The fuel gauge is present and communicating.
    Operational(OperationalSubstate),
}

/// Current internal state of the fuel gauge.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InternalState {
    /// No fuel gauge is present or it has not been initialized.
    #[default]
    NotPresent,
    /// A fuel gauge is present.
    Present(PresentSubstate),
}

/// Fuel gauge state, owned by the driver (OEM) and managed via the `on_*` transition methods.
///
/// This holds both the fuel gauge state machine state and the cached static and
/// dynamic battery data. The battery service reads this state (via
/// [`FuelGauge::state`]) when answering ACPI queries.
#[derive(Default)]
pub struct State {
    state: InternalState,
    static_cache: StaticBatteryMsgs,
    dynamic_cache: DynamicBatteryMsgs,
}

impl State {
    /// The current internal state.
    pub fn internal_state(&self) -> InternalState {
        self.state
    }

    /// A reference to the cached static battery data.
    pub fn static_cache(&self) -> &StaticBatteryMsgs {
        &self.static_cache
    }

    /// A reference to the cached dynamic battery data.
    pub fn dynamic_cache(&self) -> &DynamicBatteryMsgs {
        &self.dynamic_cache
    }

    /// Returns `true` if the fuel gauge is present.
    pub fn is_present(&self) -> bool {
        matches!(self.state, InternalState::Present(_))
    }

    /// Returns `true` if the fuel gauge is present and operational.
    pub fn is_operational(&self) -> bool {
        matches!(self.state, InternalState::Present(PresentSubstate::Operational(_)))
    }

    /// Returns `true` if the fuel gauge is present, operational, and polling.
    pub fn is_polling(&self) -> bool {
        matches!(
            self.state,
            InternalState::Present(PresentSubstate::Operational(OperationalSubstate::Polling))
        )
    }

    /// Handle fuel gauge initialization completing.
    ///
    /// Transitions to `Present(Operational(Init))`. Should be called by the
    /// driver after hardware initialization succeeds.
    pub fn on_initialized(&mut self) {
        self.state = InternalState::Present(PresentSubstate::Operational(OperationalSubstate::Init));
    }

    /// Cache freshly read static battery data.
    ///
    /// If the fuel gauge is operational this also advances the state to
    /// `Present(Operational(Polling))`. Should be called by the driver after a
    /// successful static-data read.
    pub fn on_static_data(&mut self, data: StaticBatteryMsgs) {
        self.static_cache = data;
        if self.is_operational() {
            self.state = InternalState::Present(PresentSubstate::Operational(OperationalSubstate::Polling));
        }
    }

    /// Cache freshly read dynamic battery data.
    ///
    /// Should be called by the driver after a successful dynamic-data read while
    /// in the `Present(Operational(Polling))` state.
    pub fn on_dynamic_data(&mut self, data: DynamicBatteryMsgs) {
        self.dynamic_cache = data;
    }

    /// Handle a communication timeout.
    ///
    /// Transitions a present fuel gauge to `Present(NotOperational)`. Should be
    /// called by the driver when a communication timeout is detected.
    pub fn on_timeout(&mut self) {
        if self.is_present() {
            self.state = InternalState::Present(PresentSubstate::NotOperational);
        }
    }

    /// Handle recovery after re-establishing communication.
    ///
    /// Transitions `Present(NotOperational)` back to `Present(Operational(Init))`.
    /// No-op in any other state. Should be called by the driver after a
    /// successful ping while recovering.
    pub fn on_recovered(&mut self) {
        if matches!(self.state, InternalState::Present(PresentSubstate::NotOperational)) {
            self.state = InternalState::Present(PresentSubstate::Operational(OperationalSubstate::Init));
        }
    }
}

/// Fuel gauge controller trait that device drivers implement to integrate with the battery service.
///
/// The driver is responsible for driving the fuel gauge state machine — it must
/// call the appropriate [`State`] transition methods (via [`Self::state_mut`])
/// based on hardware observations.
pub trait FuelGauge: embedded_batteries_async::smart_battery::SmartBattery {
    /// Type of error returned by the fuel gauge hardware.
    type FuelGaugeError: Into<FuelGaugeError> + embedded_batteries_async::smart_battery::Error;

    /// Initialize the fuel gauge hardware.
    ///
    /// The driver should call [`State::on_initialized`] after a successful
    /// initialization.
    fn initialize(&mut self) -> impl Future<Output = Result<(), Self::FuelGaugeError>>;

    /// Ping the fuel gauge hardware to verify communication.
    ///
    /// When used for recovery, the driver should call [`State::on_recovered`]
    /// after a successful ping.
    fn ping(&mut self) -> impl Future<Output = Result<(), Self::FuelGaugeError>>;

    /// Read static battery data from the hardware.
    ///
    /// The driver should cache the result by calling [`State::on_static_data`].
    fn update_static_data(&mut self) -> impl Future<Output = Result<(), Self::FuelGaugeError>>;

    /// Read dynamic battery data from the hardware.
    ///
    /// The driver should cache the result by calling [`State::on_dynamic_data`].
    fn update_dynamic_data(&mut self) -> impl Future<Output = Result<(), Self::FuelGaugeError>>;

    /// Return an immutable reference to the current fuel gauge state.
    fn state(&self) -> &State;

    /// Return a mutable reference to the current fuel gauge state.
    fn state_mut(&mut self) -> &mut State;
}
