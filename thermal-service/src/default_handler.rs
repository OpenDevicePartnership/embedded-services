//! ODP default MPTF handler and fan control logic which OEMs can use as-is or as a reference
use crate as thermal_service;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_services::comms;
use embedded_services::{error, info, warn};
use heapless::FnvIndexMap;
use thermal_service::fan;
use thermal_service::mptf;
use thermal_service::sensor;
use thermal_service::utils;

pub const SENSOR: sensor::DeviceId = sensor::DeviceId(0);
pub const FAN: fan::DeviceId = fan::DeviceId(0);

#[derive(Debug, Clone, Copy)]
enum FanState {
    Off,
    On,
    Ramping,
    Max,
}

#[derive(Debug, Clone)]
struct State {
    // Most recently sampled temperature
    cur_temp: f32,
    // Current fan state
    fan_state: FanState,
    // Low temperature threshold
    threshold_low: f32,
    // High temperature threshold
    threshold_high: f32,
    // Provides variable data lookup via UID
    uid_table: FnvIndexMap<mptf::Uid, mptf::Dword, 16>,
}

impl State {
    fn new(config: Config) -> Self {
        let mut uid_table = FnvIndexMap::new();

        // Unwraps used here because these are unrecoverable errors which represent a programming bug, so should panic
        uid_table
            .insert(uid::CRT_TEMP, utils::c_to_dk(config.crt_temp))
            .unwrap();

        uid_table
            .insert(uid::PROC_HOT_TEMP, utils::c_to_dk(config.proc_hot_temp))
            .unwrap();

        // Maybe unused by default?
        uid_table.insert(uid::PROFILE_TYPE, 0).unwrap();

        uid_table
            .insert(uid::FAN_ON_TEMP, utils::c_to_dk(config.fan_on_temp))
            .unwrap();

        uid_table
            .insert(uid::FAN_RAMP_TEMP, utils::c_to_dk(config.fan_ramp_temp))
            .unwrap();

        uid_table
            .insert(uid::FAN_MAX_TEMP, utils::c_to_dk(config.fan_max_temp))
            .unwrap();

        // Determined by init()
        uid_table.insert(uid::FAN_MIN_RPM, 0).unwrap();

        // Determined by init()
        uid_table.insert(uid::FAN_MAX_RPM, 0).unwrap();

        uid_table.insert(uid::FAN_CURRENT_RPM, 0).unwrap();

        Self {
            cur_temp: 0.0,
            fan_state: FanState::Off,
            threshold_low: config.threshold_low,
            threshold_high: config.threshold_high,
            uid_table,
        }
    }
}

/// Configure the default handler
pub struct Config {
    pub threshold_low: f32,
    pub threshold_high: f32,
    pub crt_temp: f32,
    pub proc_hot_temp: f32,
    pub fan_on_temp: f32,
    pub fan_ramp_temp: f32,
    pub fan_max_temp: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            threshold_low: 100.0,
            threshold_high: 100.0,
            crt_temp: 60.0,
            proc_hot_temp: 80.0,
            fan_on_temp: 30.0,
            fan_ramp_temp: 35.0,
            fan_max_temp: 40.0,
        }
    }
}

/// Default MPTF handler provided by ODP
///
/// Currently does not distinguish between separate thermal zones or cooling policies,
/// instead treating entire paltform as single zone and following a single cooling policy.
pub struct DefaultHandler {
    state: Mutex<NoopRawMutex, State>,
}

impl DefaultHandler {
    /// Create a new instance of the default MPTF handler
    pub fn new(config: Config) -> Self {
        Self {
            state: Mutex::new(State::new(config)),
        }
    }
}

impl DefaultHandler {
    pub(crate) async fn get_tmp(&self, _tzid: mptf::TzId) -> mptf::Response {
        Ok(mptf::ResponseData::GetTmp(utils::c_to_dk(
            self.state.lock().await.cur_temp,
        )))
    }

    pub(crate) async fn get_thrs(&self, _tzid: mptf::TzId) -> mptf::Response {
        let low = utils::c_to_dk(self.state.lock().await.threshold_low);
        let high = utils::c_to_dk(self.state.lock().await.threshold_high);
        Ok(mptf::ResponseData::GetThrs(0, low, high))
    }

    pub(crate) async fn set_thrs(
        &self,
        _tzid: mptf::TzId,
        _timeout: mptf::Dword,
        low: mptf::DeciKelvin,
        high: mptf::DeciKelvin,
    ) -> mptf::Response {
        // Currently default algorithm does not make use of timeout
        let low = utils::dk_to_c(low);
        let high = utils::dk_to_c(high);

        // Set thresholds on physical sensors to make use of hardware interrupts
        thermal_service::execute_sensor_request(SENSOR, sensor::Request::SetHardAlertLow(low))
            .await
            .map_err(|_| mptf::Error::HardwareError)?;
        thermal_service::execute_sensor_request(SENSOR, sensor::Request::SetHardAlertHigh(high))
            .await
            .map_err(|_| mptf::Error::HardwareError)?;

        // Then update state
        self.state.lock().await.threshold_low = low;
        self.state.lock().await.threshold_high = high;
        Ok(mptf::ResponseData::SetThrs)
    }

    pub(crate) async fn set_scp(
        &self,
        _tzid: mptf::TzId,
        _mode: mptf::Dword,
        _acoustic_lim: mptf::Dword,
        _power_lim: mptf::Dword,
    ) -> mptf::Response {
        // Currently default algorithm does not use distinct cooling policies
        Ok(mptf::ResponseData::SetScp)
    }

    pub(crate) async fn get_var(&self, uid: mptf::Uid) -> mptf::Response {
        self.state
            .lock()
            .await
            .uid_table
            .get(&uid)
            .ok_or(mptf::Error::InvalidParameter)
            .map(|&val| mptf::ResponseData::GetVar(uid, val))
    }

    pub(crate) async fn set_var(&self, uid: mptf::Uid, value: mptf::Dword) -> mptf::Response {
        let uid_table = &mut self.state.lock().await.uid_table;

        if let Some(inner_val) = uid_table.get_mut(&uid) {
            *inner_val = value;
            Ok(mptf::ResponseData::SetVar)
        } else {
            Err(mptf::Error::InvalidParameter)
        }
    }
}
