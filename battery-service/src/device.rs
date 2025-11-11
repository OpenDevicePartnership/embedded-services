use embassy_sync::{
    channel::Channel,
    mutex::{Mutex, MutexGuard},
};
use embassy_time::Duration;
use embedded_batteries_async::{
    acpi::{BmcControlFlags, BmdCapabilityFlags, BmdStatusFlags, PowerThresholdSupport},
    smart_battery::BatteryModeFields,
};
use embedded_services::{GlobalRawMutex, Node, NodeContainer, SyncCell};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Device errors.
pub enum FuelGaugeError {
    Timeout,
    BusError,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Device commands.
pub enum Command {
    Initialize,
    Ping,
    UpdateStaticCache,
    UpdateDynamicCache,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Device response.
pub enum InternalResponse {
    Complete,
}

/// External device response.
pub type Response = Result<InternalResponse, FuelGaugeError>;

/// Standard static battery data cache
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

/// Standard dynamic battery data cache
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

/// Fuel gauge ID
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Hardware agnostic device object to be registered with context.
pub struct Device {
    node: embedded_services::Node,
    id: DeviceId,
    command: Channel<GlobalRawMutex, Command, 1>,
    response: Channel<GlobalRawMutex, Response, 1>,
    dynamic_battery_cache: Mutex<GlobalRawMutex, DynamicBatteryMsgs>,
    static_battery_cache: Mutex<GlobalRawMutex, StaticBatteryMsgs>,
    timeout: SyncCell<Duration>,
}

impl Device {
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: embedded_services::Node::uninit(),
            id,
            command: Channel::new(),
            response: Channel::new(),
            dynamic_battery_cache: Mutex::default(),
            static_battery_cache: Mutex::default(),
            timeout: SyncCell::new(Duration::from_secs(60)),
        }
    }

    /// Get device ID.
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Send command to the device.
    pub async fn send_command(&self, cmd: Command) {
        self.command.send(cmd).await
    }

    /// Wait for a response from the device.
    pub async fn wait_response(&self) -> Response {
        self.response.receive().await
    }

    /// Send a command and wait for a response from the device.
    pub async fn execute_command(&self, cmd: Command) -> Response {
        self.send_command(cmd).await;
        self.wait_response().await
    }

    /// Receive a command.
    pub async fn receive_command(&self) -> Command {
        self.command.receive().await
    }

    /// Send a response.
    pub async fn send_response(&self, response: Response) {
        self.response.send(response).await
    }

    /// Set dynamic battery cache with updated values.
    pub async fn set_dynamic_battery_cache(&self, new_values: DynamicBatteryMsgs) {
        *self.dynamic_battery_cache.lock().await = new_values;
    }

    /// Set static battery cache with updated values.
    pub async fn set_static_battery_cache(&self, new_values: StaticBatteryMsgs) {
        *self.static_battery_cache.lock().await = new_values;
    }

    /// Get dynamic battery cache.
    pub async fn get_dynamic_battery_cache(&self) -> DynamicBatteryMsgs {
        *self.dynamic_battery_cache.lock().await
    }

    /// Get static battery cache.
    pub async fn get_static_battery_cache(&self) -> StaticBatteryMsgs {
        *self.static_battery_cache.lock().await
    }

    /// Get static battery cache by grabbing the lock.
    ///
    /// WARNING: More performant than get_static_battery_cache,
    /// but drop the MutexGuard before the next await point to avoid deadlocks!
    pub async fn get_static_battery_cache_guarded(&self) -> MutexGuard<'_, GlobalRawMutex, StaticBatteryMsgs> {
        self.static_battery_cache.lock().await
    }

    /// Get dynamic battery cache by grabbing the lock.
    ///
    /// WARNING: More performant than get_dynamic_battery_cache,
    /// but drop the MutexGuard before the next await point to avoid deadlocks!
    pub async fn get_dynamic_battery_cache_guarded(&self) -> MutexGuard<'_, GlobalRawMutex, DynamicBatteryMsgs> {
        self.dynamic_battery_cache.lock().await
    }

    /// Set device timeout.
    pub fn set_timeout(&self, duration: Duration) {
        self.timeout.set(duration);
    }

    /// Get device timeout.
    pub fn get_timeout(&self) -> Duration {
        self.timeout.get()
    }
}

impl NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
    }
}
