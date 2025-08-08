//! EC Internal Data Structures

#[allow(missing_docs)]
pub const EC_MEMMAP_VERSION: Version = Version {
    major: 0,
    minor: 1,
    spin: 0,
    res0: 0,
};

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub spin: u8,
    pub res0: u8,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    pub events: u32,
    pub fw_version: Version,
    pub secure_state: u8,
    pub boot_status: u8,
    pub fan_mask: u8,
    pub battery_mask: u8,
    pub temp_mask: u16,
    pub key_mask: u16,
    pub debug_mask: u16,
    pub res0: u16,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeAlarm {
    pub events: u32,
    pub capability: u32,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub valid: u8,
    pub daylight: u8,
    pub res1: u8,
    pub milli: u16,
    pub time_zone: u16,
    pub res2: u16,
    pub alarm_status: u32,
    pub ac_time_val: u32,
    pub dc_time_val: u32,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Battery {
    pub events: u32,
    pub status: u32,
    pub last_full_charge: u32,
    pub cycle_count: u32,
    pub state: u32,
    pub present_rate: u32,
    pub remain_cap: u32,
    pub present_volt: u32,
    pub psr_state: u32,
    pub psr_max_out: u32,
    pub psr_max_in: u32,
    pub peak_level: u32,
    pub peak_power: u32,
    pub sus_level: u32,
    pub sus_power: u32,
    pub peak_thres: u32,
    pub sus_thres: u32,
    pub trip_thres: u32,
    pub bmc_data: u32,
    pub bmd_data: u32,
    pub bmd_flags: u32,
    pub bmd_count: u32,
    pub charge_time: u32,
    pub run_time: u32,
    pub sample_time: u32,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Thermal {
    pub events: u32,
    pub cool_mode: u32,
    pub dba_limit: u32,
    pub sonne_limit: u32,
    pub ma_limit: u32,
    pub fan1_on_temp: u32,
    pub fan1_ramp_temp: u32,
    pub fan1_max_temp: u32,
    pub fan1_crt_temp: u32,
    pub fan1_hot_temp: u32,
    pub fan1_max_rpm: u32,
    pub fan1_cur_rpm: u32,
    pub tmp1_val: u32,
    pub tmp1_timeout: u32,
    pub tmp1_low: u32,
    pub tmp1_high: u32,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct Debug {
    pub status: u32,
    pub buffer_offset: u32,
    pub buffer_length: u32,
    pub data: [u8; 128],
}

impl Default for Debug {
    fn default() -> Self {
        Self {
            status: 0,
            buffer_offset: 0,
            buffer_length: 0,
            data: [0; 128],
        }
    }
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Notifications {
    pub service: u16,
    pub event: u16,
}

#[allow(missing_docs)]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ECMemory {
    pub ver: Version,
    pub caps: Capabilities,
    pub notif: Notifications,
    pub alarm: TimeAlarm,
    pub batt: Battery,
    pub therm: Thermal,
    pub debug: Debug,
}
