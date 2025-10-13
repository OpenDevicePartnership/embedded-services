//! Standard EC types
use core::mem::offset_of;

pub mod message;
pub mod protocols;
pub mod structure;

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The requested base + offset is invalid
    InvalidLocation,
}

/// Update battery section of memory map based on battery message
pub fn update_battery_section(msg: &message::BatteryMessage, memory_map: &mut structure::ECMemory) {
    match msg {
        message::BatteryMessage::Events(events) => memory_map.batt.events = *events,
        message::BatteryMessage::Status(status) => memory_map.batt.status = *status,
        message::BatteryMessage::LastFullCharge(last_full_charge) => {
            memory_map.batt.last_full_charge = *last_full_charge
        }
        message::BatteryMessage::CycleCount(cycle_count) => memory_map.batt.cycle_count = *cycle_count,
        message::BatteryMessage::State(state) => memory_map.batt.state = *state,
        message::BatteryMessage::PresentRate(present_rate) => memory_map.batt.present_rate = *present_rate,
        message::BatteryMessage::RemainCap(remain_cap) => memory_map.batt.remain_cap = *remain_cap,
        message::BatteryMessage::PresentVolt(present_volt) => memory_map.batt.present_volt = *present_volt,
        message::BatteryMessage::PsrState(psr_state) => memory_map.batt.psr_state = *psr_state,
        message::BatteryMessage::PsrMaxOut(psr_max_out) => memory_map.batt.psr_max_out = *psr_max_out,
        message::BatteryMessage::PsrMaxIn(psr_max_in) => memory_map.batt.psr_max_in = *psr_max_in,
        message::BatteryMessage::PeakLevel(peak_level) => memory_map.batt.peak_level = *peak_level,
        message::BatteryMessage::PeakPower(peak_power) => memory_map.batt.peak_power = *peak_power,
        message::BatteryMessage::SusLevel(sus_level) => memory_map.batt.sus_level = *sus_level,
        message::BatteryMessage::SusPower(sus_power) => memory_map.batt.sus_power = *sus_power,
        message::BatteryMessage::PeakThres(peak_thres) => memory_map.batt.peak_thres = *peak_thres,
        message::BatteryMessage::SusThres(sus_thres) => memory_map.batt.sus_thres = *sus_thres,
        message::BatteryMessage::TripThres(trip_thres) => memory_map.batt.trip_thres = *trip_thres,
        message::BatteryMessage::BmcData(bmc_data) => memory_map.batt.bmc_data = *bmc_data,
        message::BatteryMessage::BmdData(bmd_data) => memory_map.batt.bmd_data = *bmd_data,
        message::BatteryMessage::BmdFlags(bmd_flags) => memory_map.batt.bmd_flags = *bmd_flags,
        message::BatteryMessage::BmdCount(bmd_count) => memory_map.batt.bmd_count = *bmd_count,
        message::BatteryMessage::ChargeTime(charge_time) => memory_map.batt.charge_time = *charge_time,
        message::BatteryMessage::RunTime(run_time) => memory_map.batt.run_time = *run_time,
        message::BatteryMessage::SampleTime(sample_time) => memory_map.batt.sample_time = *sample_time,
    }
}

/// Update capabilities section of memory map based on battery message
pub fn update_capabilities_section(msg: &message::CapabilitiesMessage, memory_map: &mut structure::ECMemory) {
    match msg {
        message::CapabilitiesMessage::Events(events) => memory_map.caps.events = *events,
        message::CapabilitiesMessage::FwVersion(fw_version) => memory_map.caps.fw_version = *fw_version,
        message::CapabilitiesMessage::SecureState(secure_state) => memory_map.caps.secure_state = *secure_state,
        message::CapabilitiesMessage::BootStatus(boot_status) => memory_map.caps.boot_status = *boot_status,
        message::CapabilitiesMessage::FanMask(fan_mask) => memory_map.caps.fan_mask = *fan_mask,
        message::CapabilitiesMessage::BatteryMask(battery_mask) => memory_map.caps.battery_mask = *battery_mask,
        message::CapabilitiesMessage::TempMask(temp_mask) => memory_map.caps.temp_mask = *temp_mask,
        message::CapabilitiesMessage::KeyMask(key_mask) => memory_map.caps.key_mask = *key_mask,
        message::CapabilitiesMessage::DebugMask(debug_mask) => memory_map.caps.debug_mask = *debug_mask,
    }
}

/// Update thermal section of memory map based on battery message
pub fn update_thermal_section(msg: &message::ThermalMessage, memory_map: &mut structure::ECMemory) {
    match msg {
        message::ThermalMessage::Events(events) => memory_map.therm.events = *events,
        message::ThermalMessage::CoolMode(cool_mode) => memory_map.therm.cool_mode = *cool_mode,
        message::ThermalMessage::DbaLimit(dba_limit) => memory_map.therm.dba_limit = *dba_limit,
        message::ThermalMessage::SonneLimit(sonne_limit) => memory_map.therm.sonne_limit = *sonne_limit,
        message::ThermalMessage::MaLimit(ma_limit) => memory_map.therm.ma_limit = *ma_limit,
        message::ThermalMessage::Fan1OnTemp(fan1_on_temp) => memory_map.therm.fan1_on_temp = *fan1_on_temp,
        message::ThermalMessage::Fan1RampTemp(fan1_ramp_temp) => memory_map.therm.fan1_ramp_temp = *fan1_ramp_temp,
        message::ThermalMessage::Fan1MaxTemp(fan1_max_temp) => memory_map.therm.fan1_max_temp = *fan1_max_temp,
        message::ThermalMessage::Fan1CrtTemp(fan1_crt_temp) => memory_map.therm.fan1_crt_temp = *fan1_crt_temp,
        message::ThermalMessage::Fan1HotTemp(fan1_hot_temp) => memory_map.therm.fan1_hot_temp = *fan1_hot_temp,
        message::ThermalMessage::Fan1MaxRpm(fan1_max_rpm) => memory_map.therm.fan1_max_rpm = *fan1_max_rpm,
        message::ThermalMessage::Fan1CurRpm(fan1_cur_rpm) => memory_map.therm.fan1_cur_rpm = *fan1_cur_rpm,
        message::ThermalMessage::Tmp1Val(tmp1_val) => memory_map.therm.tmp1_val = *tmp1_val,
        message::ThermalMessage::Tmp1Timeout(tmp1_timeout) => memory_map.therm.tmp1_timeout = *tmp1_timeout,
        message::ThermalMessage::Tmp1Low(tmp1_low) => memory_map.therm.tmp1_low = *tmp1_low,
        message::ThermalMessage::Tmp1High(tmp1_high) => memory_map.therm.tmp1_high = *tmp1_high,
    }
}

/// Helper macro to simplify the conversion of memory map to message
macro_rules! into_message {
    ($offset:ident, $length:ident, $member:expr, $msg:expr) => {
        let value = $member;
        *$offset += size_of_val(&value);
        *$length -= size_of_val(&value);
        return Ok($msg(value));
    };
}

/// Convert from memory map offset and length to battery message
/// Modifies offset and length
pub fn mem_map_to_battery_msg(
    memory_map: &structure::ECMemory,
    offset: &mut usize,
    length: &mut usize,
) -> Result<message::BatteryMessage, Error> {
    let local_offset = *offset - offset_of!(structure::ECMemory, batt);

    if local_offset == offset_of!(structure::Battery, events) {
        into_message!(offset, length, memory_map.batt.events, message::BatteryMessage::Events);
    } else if local_offset == offset_of!(structure::Battery, status) {
        into_message!(offset, length, memory_map.batt.status, message::BatteryMessage::Status);
    } else if local_offset == offset_of!(structure::Battery, last_full_charge) {
        into_message!(
            offset,
            length,
            memory_map.batt.last_full_charge,
            message::BatteryMessage::LastFullCharge
        );
    } else if local_offset == offset_of!(structure::Battery, cycle_count) {
        into_message!(
            offset,
            length,
            memory_map.batt.cycle_count,
            message::BatteryMessage::CycleCount
        );
    } else if local_offset == offset_of!(structure::Battery, state) {
        into_message!(offset, length, memory_map.batt.state, message::BatteryMessage::State);
    } else if local_offset == offset_of!(structure::Battery, present_rate) {
        into_message!(
            offset,
            length,
            memory_map.batt.present_rate,
            message::BatteryMessage::PresentRate
        );
    } else if local_offset == offset_of!(structure::Battery, remain_cap) {
        into_message!(
            offset,
            length,
            memory_map.batt.remain_cap,
            message::BatteryMessage::RemainCap
        );
    } else if local_offset == offset_of!(structure::Battery, present_volt) {
        into_message!(
            offset,
            length,
            memory_map.batt.present_volt,
            message::BatteryMessage::PresentVolt
        );
    } else if local_offset == offset_of!(structure::Battery, psr_state) {
        into_message!(
            offset,
            length,
            memory_map.batt.psr_state,
            message::BatteryMessage::PsrState
        );
    } else if local_offset == offset_of!(structure::Battery, psr_max_out) {
        into_message!(
            offset,
            length,
            memory_map.batt.psr_max_out,
            message::BatteryMessage::PsrMaxOut
        );
    } else if local_offset == offset_of!(structure::Battery, psr_max_in) {
        into_message!(
            offset,
            length,
            memory_map.batt.psr_max_in,
            message::BatteryMessage::PsrMaxIn
        );
    } else if local_offset == offset_of!(structure::Battery, peak_level) {
        into_message!(
            offset,
            length,
            memory_map.batt.peak_level,
            message::BatteryMessage::PeakLevel
        );
    } else if local_offset == offset_of!(structure::Battery, peak_power) {
        into_message!(
            offset,
            length,
            memory_map.batt.peak_power,
            message::BatteryMessage::PeakPower
        );
    } else if local_offset == offset_of!(structure::Battery, sus_level) {
        into_message!(
            offset,
            length,
            memory_map.batt.sus_level,
            message::BatteryMessage::SusLevel
        );
    } else if local_offset == offset_of!(structure::Battery, sus_power) {
        into_message!(
            offset,
            length,
            memory_map.batt.sus_power,
            message::BatteryMessage::SusPower
        );
    } else if local_offset == offset_of!(structure::Battery, peak_thres) {
        into_message!(
            offset,
            length,
            memory_map.batt.peak_thres,
            message::BatteryMessage::PeakThres
        );
    } else if local_offset == offset_of!(structure::Battery, sus_thres) {
        into_message!(
            offset,
            length,
            memory_map.batt.sus_thres,
            message::BatteryMessage::SusThres
        );
    } else if local_offset == offset_of!(structure::Battery, trip_thres) {
        into_message!(
            offset,
            length,
            memory_map.batt.trip_thres,
            message::BatteryMessage::TripThres
        );
    } else if local_offset == offset_of!(structure::Battery, bmc_data) {
        into_message!(
            offset,
            length,
            memory_map.batt.bmc_data,
            message::BatteryMessage::BmcData
        );
    } else if local_offset == offset_of!(structure::Battery, bmd_data) {
        into_message!(
            offset,
            length,
            memory_map.batt.bmd_data,
            message::BatteryMessage::BmdData
        );
    } else if local_offset == offset_of!(structure::Battery, bmd_flags) {
        into_message!(
            offset,
            length,
            memory_map.batt.bmd_flags,
            message::BatteryMessage::BmdFlags
        );
    } else if local_offset == offset_of!(structure::Battery, bmd_count) {
        into_message!(
            offset,
            length,
            memory_map.batt.bmd_count,
            message::BatteryMessage::BmdCount
        );
    } else if local_offset == offset_of!(structure::Battery, charge_time) {
        into_message!(
            offset,
            length,
            memory_map.batt.charge_time,
            message::BatteryMessage::ChargeTime
        );
    } else if local_offset == offset_of!(structure::Battery, run_time) {
        into_message!(
            offset,
            length,
            memory_map.batt.run_time,
            message::BatteryMessage::RunTime
        );
    } else if local_offset == offset_of!(structure::Battery, sample_time) {
        into_message!(
            offset,
            length,
            memory_map.batt.sample_time,
            message::BatteryMessage::SampleTime
        );
    } else {
        Err(Error::InvalidLocation)
    }
}

/// Convert from memory map offset and length to thermal message
/// Modifies offset and length
pub fn mem_map_to_thermal_msg(
    memory_map: &structure::ECMemory,
    offset: &mut usize,
    length: &mut usize,
) -> Result<message::ThermalMessage, Error> {
    let local_offset = *offset - offset_of!(structure::ECMemory, therm);

    if local_offset == offset_of!(structure::Thermal, events) {
        into_message!(offset, length, memory_map.therm.events, message::ThermalMessage::Events);
    } else if local_offset == offset_of!(structure::Thermal, cool_mode) {
        into_message!(
            offset,
            length,
            memory_map.therm.cool_mode,
            message::ThermalMessage::CoolMode
        );
    } else if local_offset == offset_of!(structure::Thermal, dba_limit) {
        into_message!(
            offset,
            length,
            memory_map.therm.dba_limit,
            message::ThermalMessage::DbaLimit
        );
    } else if local_offset == offset_of!(structure::Thermal, sonne_limit) {
        into_message!(
            offset,
            length,
            memory_map.therm.sonne_limit,
            message::ThermalMessage::SonneLimit
        );
    } else if local_offset == offset_of!(structure::Thermal, ma_limit) {
        into_message!(
            offset,
            length,
            memory_map.therm.ma_limit,
            message::ThermalMessage::MaLimit
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_on_temp) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_on_temp,
            message::ThermalMessage::Fan1OnTemp
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_ramp_temp) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_ramp_temp,
            message::ThermalMessage::Fan1RampTemp
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_max_temp) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_max_temp,
            message::ThermalMessage::Fan1MaxTemp
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_crt_temp) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_crt_temp,
            message::ThermalMessage::Fan1CrtTemp
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_hot_temp) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_hot_temp,
            message::ThermalMessage::Fan1HotTemp
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_max_rpm) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_max_rpm,
            message::ThermalMessage::Fan1MaxRpm
        );
    } else if local_offset == offset_of!(structure::Thermal, fan1_cur_rpm) {
        into_message!(
            offset,
            length,
            memory_map.therm.fan1_cur_rpm,
            message::ThermalMessage::Fan1CurRpm
        );
    } else if local_offset == offset_of!(structure::Thermal, tmp1_val) {
        into_message!(
            offset,
            length,
            memory_map.therm.tmp1_val,
            message::ThermalMessage::Tmp1Val
        );
    } else if local_offset == offset_of!(structure::Thermal, tmp1_timeout) {
        into_message!(
            offset,
            length,
            memory_map.therm.tmp1_timeout,
            message::ThermalMessage::Tmp1Timeout
        );
    } else if local_offset == offset_of!(structure::Thermal, tmp1_low) {
        into_message!(
            offset,
            length,
            memory_map.therm.tmp1_low,
            message::ThermalMessage::Tmp1Low
        );
    } else if local_offset == offset_of!(structure::Thermal, tmp1_high) {
        into_message!(
            offset,
            length,
            memory_map.therm.tmp1_high,
            message::ThermalMessage::Tmp1High
        );
    } else {
        Err(Error::InvalidLocation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_field {
        ($memory_map:ident, $offset:ident, $length:ident, $field:expr, $func:ident, $msg:expr) => {
            let field = $field;
            let next_offset = $offset + size_of_val(&field);
            let next_length = $length - size_of_val(&field);
            let msg = $func(&$memory_map, &mut $offset, &mut $length).unwrap();
            assert_eq!(msg, $msg(field));
            assert_eq!($offset, next_offset);
            assert_eq!($length, next_length);
        };
    }

    #[test]
    fn test_mem_map_to_battery_msg() {
        use crate::ec_type::message::BatteryMessage;
        use crate::ec_type::structure::{Battery, ECMemory};

        let memory_map = ECMemory {
            batt: Battery {
                events: 1,
                status: 2,
                last_full_charge: 3,
                cycle_count: 4,
                state: 5,
                present_rate: 6,
                remain_cap: 7,
                present_volt: 8,
                psr_state: 9,
                psr_max_out: 10,
                psr_max_in: 11,
                peak_level: 12,
                peak_power: 13,
                sus_level: 14,
                sus_power: 15,
                peak_thres: 16,
                sus_thres: 17,
                trip_thres: 18,
                bmc_data: 19,
                bmd_data: 20,
                bmd_flags: 21,
                bmd_count: 22,
                charge_time: 23,
                run_time: 24,
                sample_time: 25,
            },
            ..Default::default()
        };

        let mut offset = offset_of!(ECMemory, batt);
        let mut length = size_of::<Battery>();

        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.events,
            mem_map_to_battery_msg,
            BatteryMessage::Events
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.status,
            mem_map_to_battery_msg,
            BatteryMessage::Status
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.last_full_charge,
            mem_map_to_battery_msg,
            BatteryMessage::LastFullCharge
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.cycle_count,
            mem_map_to_battery_msg,
            BatteryMessage::CycleCount
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.state,
            mem_map_to_battery_msg,
            BatteryMessage::State
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.present_rate,
            mem_map_to_battery_msg,
            BatteryMessage::PresentRate
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.remain_cap,
            mem_map_to_battery_msg,
            BatteryMessage::RemainCap
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.present_volt,
            mem_map_to_battery_msg,
            BatteryMessage::PresentVolt
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.psr_state,
            mem_map_to_battery_msg,
            BatteryMessage::PsrState
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.psr_max_out,
            mem_map_to_battery_msg,
            BatteryMessage::PsrMaxOut
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.psr_max_in,
            mem_map_to_battery_msg,
            BatteryMessage::PsrMaxIn
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.peak_level,
            mem_map_to_battery_msg,
            BatteryMessage::PeakLevel
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.peak_power,
            mem_map_to_battery_msg,
            BatteryMessage::PeakPower
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.sus_level,
            mem_map_to_battery_msg,
            BatteryMessage::SusLevel
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.sus_power,
            mem_map_to_battery_msg,
            BatteryMessage::SusPower
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.peak_thres,
            mem_map_to_battery_msg,
            BatteryMessage::PeakThres
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.sus_thres,
            mem_map_to_battery_msg,
            BatteryMessage::SusThres
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.trip_thres,
            mem_map_to_battery_msg,
            BatteryMessage::TripThres
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.bmc_data,
            mem_map_to_battery_msg,
            BatteryMessage::BmcData
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.bmd_data,
            mem_map_to_battery_msg,
            BatteryMessage::BmdData
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.bmd_flags,
            mem_map_to_battery_msg,
            BatteryMessage::BmdFlags
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.bmd_count,
            mem_map_to_battery_msg,
            BatteryMessage::BmdCount
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.charge_time,
            mem_map_to_battery_msg,
            BatteryMessage::ChargeTime
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.run_time,
            mem_map_to_battery_msg,
            BatteryMessage::RunTime
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.batt.sample_time,
            mem_map_to_battery_msg,
            BatteryMessage::SampleTime
        );

        assert_eq!(length, 0);
    }

    #[test]
    fn test_mem_map_to_battery_msg_error() {
        use crate::ec_type::structure::{Battery, ECMemory};

        let memory_map = ECMemory {
            batt: Battery {
                events: 1,
                status: 2,
                last_full_charge: 3,
                cycle_count: 4,
                state: 5,
                present_rate: 6,
                remain_cap: 7,
                present_volt: 8,
                psr_state: 9,
                psr_max_out: 10,
                psr_max_in: 11,
                peak_level: 12,
                peak_power: 13,
                sus_level: 14,
                sus_power: 15,
                peak_thres: 16,
                sus_thres: 17,
                trip_thres: 18,
                bmc_data: 19,
                bmd_data: 20,
                bmd_flags: 21,
                bmd_count: 22,
                charge_time: 23,
                run_time: 24,
                sample_time: 25,
            },
            ..Default::default()
        };

        let mut offset = offset_of!(ECMemory, batt) + 1;
        let mut length = size_of::<Battery>();

        let res = mem_map_to_battery_msg(&memory_map, &mut offset, &mut length);
        assert!(res.is_err() && res.unwrap_err() == Error::InvalidLocation);
    }

    #[test]
    fn test_mem_map_to_thermal_msg() {
        use crate::ec_type::message::ThermalMessage;
        use crate::ec_type::structure::{ECMemory, Thermal};

        let memory_map = ECMemory {
            therm: Thermal {
                events: 1,
                cool_mode: 2,
                dba_limit: 3,
                sonne_limit: 4,
                ma_limit: 5,
                fan1_on_temp: 6,
                fan1_ramp_temp: 7,
                fan1_max_temp: 8,
                fan1_crt_temp: 9,
                fan1_hot_temp: 10,
                fan1_max_rpm: 11,
                fan1_cur_rpm: 12,
                tmp1_val: 13,
                tmp1_timeout: 14,
                tmp1_low: 15,
                tmp1_high: 16,
            },
            ..Default::default()
        };

        let mut offset = offset_of!(ECMemory, therm);
        let mut length = size_of::<Thermal>();

        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.events,
            mem_map_to_thermal_msg,
            ThermalMessage::Events
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.cool_mode,
            mem_map_to_thermal_msg,
            ThermalMessage::CoolMode
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.dba_limit,
            mem_map_to_thermal_msg,
            ThermalMessage::DbaLimit
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.sonne_limit,
            mem_map_to_thermal_msg,
            ThermalMessage::SonneLimit
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.ma_limit,
            mem_map_to_thermal_msg,
            ThermalMessage::MaLimit
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_on_temp,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1OnTemp
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_ramp_temp,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1RampTemp
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_max_temp,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1MaxTemp
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_crt_temp,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1CrtTemp
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_hot_temp,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1HotTemp
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_max_rpm,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1MaxRpm
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.fan1_cur_rpm,
            mem_map_to_thermal_msg,
            ThermalMessage::Fan1CurRpm
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.tmp1_val,
            mem_map_to_thermal_msg,
            ThermalMessage::Tmp1Val
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.tmp1_timeout,
            mem_map_to_thermal_msg,
            ThermalMessage::Tmp1Timeout
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.tmp1_low,
            mem_map_to_thermal_msg,
            ThermalMessage::Tmp1Low
        );
        test_field!(
            memory_map,
            offset,
            length,
            memory_map.therm.tmp1_high,
            mem_map_to_thermal_msg,
            ThermalMessage::Tmp1High
        );

        assert_eq!(length, 0);
    }

    #[test]
    fn test_mem_map_to_thermal_msg_error() {
        use crate::ec_type::structure::{ECMemory, Thermal};

        let memory_map = ECMemory {
            therm: Thermal {
                events: 1,
                cool_mode: 2,
                dba_limit: 3,
                sonne_limit: 4,
                ma_limit: 5,
                fan1_on_temp: 6,
                fan1_ramp_temp: 7,
                fan1_max_temp: 8,
                fan1_crt_temp: 9,
                fan1_hot_temp: 10,
                fan1_max_rpm: 11,
                fan1_cur_rpm: 12,
                tmp1_val: 13,
                tmp1_timeout: 14,
                tmp1_low: 15,
                tmp1_high: 16,
            },
            ..Default::default()
        };

        let mut offset = offset_of!(ECMemory, therm) + 1;
        let mut length = size_of::<Thermal>();

        let res = mem_map_to_thermal_msg(&memory_map, &mut offset, &mut length);
        assert!(res.is_err() && res.unwrap_err() == Error::InvalidLocation);
    }
}
