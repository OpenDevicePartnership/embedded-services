//! ODP default MPTF handler and fan control logic which OEMs can use as-is or as a reference
use crate as thermal_service;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_services::comms;
use embedded_services::{error, info, warn};
use heapless::FnvIndexMap;
use mptf::Handler;
use thermal_service::fan;
use thermal_service::mptf;
use thermal_service::sensor;
use thermal_service::utils;

pub const SENSOR: sensor::DeviceId = sensor::DeviceId(0);
pub const FAN: fan::DeviceId = fan::DeviceId(0);

/// MPTF Standard UIDs which the default service understands
// TODO: Put these UIDs into the actual correct format according to spec
pub mod uid {
    use crate::mptf::Uid;
    pub const CRT_TEMP: Uid = 0x218246e7baf645f1aa1307e4845256b8;
    pub const PROC_HOT_TEMP: Uid = 0x22dc52d2fd0b47ab95b826552f9831a5;
    pub const PROFILE_TYPE: Uid = 0x23b4a025cdfd4af9a41137a24c574615;
    pub const FAN_ON_TEMP: Uid = 0xba17b567c36848d5bc6fa312a41583c1;
    pub const FAN_RAMP_TEMP: Uid = 0x3a62688cd95b4d2dbacc90d7a5816bcd;
    pub const FAN_MAX_TEMP: Uid = 0xdcb758b1f0fd4ec7b2c0ef1e2a547b76;
    pub const FAN_MIN_RPM: Uid = 0xdb261c77934b45e29742256c62badb7a;
    pub const FAN_MAX_RPM: Uid = 0x5cf839df8be742b99ac53403ca2c8a6a;
    pub const FAN_CURRENT_RPM: Uid = 0xadf9549207764ffc84f3b6c8b5269683;
}

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

    /// Initializes the default handler using values retrieved from previously registered hardware
    pub async fn init(&self) -> Result<(), ()> {
        // Request fan min and max RPM from device
        let min_rpm = match thermal_service::execute_fan_request(FAN, fan::Request::GetMinRpm).await {
            Ok(fan::ResponseData::Rpm(rpm)) => rpm,
            _ => return Err(()),
        };
        let max_rpm = match thermal_service::execute_fan_request(FAN, fan::Request::GetMaxRpm).await {
            Ok(fan::ResponseData::Rpm(rpm)) => rpm,
            _ => return Err(()),
        };

        // Then update UID table with that info
        self.state.lock().await.uid_table[&uid::FAN_MIN_RPM] = min_rpm as u32;
        self.state.lock().await.uid_table[&uid::FAN_MAX_RPM] = max_rpm as u32;

        // Ensure sensor is configured to generate alerts with initial threshold config
        thermal_service::execute_sensor_request(
            SENSOR,
            sensor::Request::SetAlertLow(self.state.lock().await.threshold_low),
        )
        .await
        .map_err(|_| ())?;

        thermal_service::execute_sensor_request(
            SENSOR,
            sensor::Request::SetAlertHigh(self.state.lock().await.threshold_high),
        )
        .await
        .map_err(|_| ())?;

        Ok(())
    }

    // TODO: Make this possibly a callback so OEM can inject a custom ramp response but keep other boilerplate?
    // TODO: Or make it a trait method? Investigate.
    async fn fan_ramp_response(&self) -> fan::Response {
        let state = self.state.lock().await;
        let fan_ramp_tamp = utils::dk_to_c(state.uid_table[&uid::FAN_RAMP_TEMP]);
        let fan_max_tamp = utils::dk_to_c(state.uid_table[&uid::FAN_MAX_TEMP]);

        // Provide a linear fan response between its min and max RPM relative to temperature between ramp start and max temp
        let rpm = if state.cur_temp <= fan_ramp_tamp {
            state.uid_table[&uid::FAN_MIN_RPM]
        } else if state.cur_temp >= fan_max_tamp {
            state.uid_table[&uid::FAN_MAX_RPM]
        } else {
            let ratio = (state.cur_temp - fan_ramp_tamp) / (fan_max_tamp - fan_ramp_tamp);
            let range = (state.uid_table[&uid::FAN_MAX_RPM] - state.uid_table[&uid::FAN_MIN_RPM]) as f32;
            state.uid_table[&uid::FAN_MIN_RPM] + (ratio * range) as u32
        };

        thermal_service::execute_fan_request(FAN, fan::Request::SetRpm(rpm as u16)).await
    }

    async fn handle_fan_off_state(&self) -> fan::Response {
        let mut state = self.state.lock().await;

        // If temp rises above Fan Min On Temp, set fan to ON state
        if state.cur_temp >= utils::dk_to_c(state.uid_table[&uid::FAN_ON_TEMP]) {
            let _ = thermal_service::execute_fan_request(
                FAN,
                fan::Request::SetRpm(state.uid_table[&uid::FAN_MIN_RPM] as u16),
            )
            .await?;
            state.fan_state = FanState::On;
            info!("Fan transitioned to ON state from OFF state");
        }

        Ok(fan::ResponseData::Success)
    }

    async fn handle_fan_on_state(&self) -> fan::Response {
        let mut state = self.state.lock().await;

        // If temp rises above Fan Ramp Temp, set to RAMPING state
        if state.cur_temp >= utils::dk_to_c(state.uid_table[&uid::FAN_RAMP_TEMP]) {
            state.fan_state = FanState::Ramping;
            info!("Fan transitioned to RAMPING state from ON state");

        // If falls below on temp, set to OFF state
        } else if state.cur_temp < utils::dk_to_c(state.uid_table[&uid::FAN_ON_TEMP]) {
            let _ = thermal_service::execute_fan_request(FAN, fan::Request::SetRpm(0)).await?;
            state.fan_state = FanState::Off;
            info!("Fan transitioned to OFF state from ON state");
        }

        Ok(fan::ResponseData::Success)
    }

    async fn handle_fan_ramping_state(&self) -> fan::Response {
        let mut state = self.state.lock().await;

        // If temp falls below ramp temp, set to ON state
        if state.cur_temp < utils::dk_to_c(state.uid_table[&uid::FAN_RAMP_TEMP]) {
            let _ = thermal_service::execute_fan_request(
                FAN,
                fan::Request::SetRpm(state.uid_table[&uid::FAN_MIN_RPM] as u16),
            )
            .await?;
            state.fan_state = FanState::On;
            info!("Fan transitioned to ON state from RAMPING state");

        // If temp rises above max, set to MAX state
        } else if state.cur_temp >= utils::dk_to_c(state.uid_table[&uid::FAN_MAX_TEMP]) {
            let _ = thermal_service::execute_fan_request(
                FAN,
                fan::Request::SetRpm(state.uid_table[&uid::FAN_MAX_RPM] as u16),
            )
            .await?;

            state.fan_state = FanState::Max;
            warn!("Fan transitioned to MAX state from RAMPING state");

        // If temp stays between ramp temp and max temp, continue ramp response
        } else {
            drop(state); // Allow fan_ramp_response to acquire state
            let _ = self.fan_ramp_response().await?;
        }

        Ok(fan::ResponseData::Success)
    }

    async fn handle_fan_max_state(&self) -> fan::Response {
        let mut state = self.state.lock().await;

        if state.cur_temp < utils::dk_to_c(state.uid_table[&uid::FAN_MAX_TEMP]) {
            state.fan_state = FanState::Ramping;
            info!("Fan transitioned to RAMPING state from MAX state");
        }

        Ok(fan::ResponseData::Success)
    }

    async fn handle_fan_state(&self) -> fan::Response {
        let state = self.state.lock().await.fan_state;

        match state {
            FanState::Off => self.handle_fan_off_state().await,
            FanState::On => self.handle_fan_on_state().await,
            FanState::Ramping => self.handle_fan_ramping_state().await,
            FanState::Max => self.handle_fan_max_state().await,
        }
    }

    // This currently just polls temperature to check if above CRT and PROCHOT (interrupt alerts are reserved for THRS)
    // TODO: Get more clarification on CRT and PROCHOT. Might not be necessary to notify?
    async fn crt_prochot_check(&self) {
        let state = self.state.lock().await;

        // If temp rises above PROCHOT, notify Host
        if state.cur_temp >= utils::dk_to_c(state.uid_table[&uid::PROC_HOT_TEMP]) {
            thermal_service::send_service_msg(
                comms::EndpointID::External(comms::External::Host),
                &mptf::Notify::ProcHot,
            )
            .await;

            warn!("Temperature exceeds PROCHOT!");
        }

        // If temp rises above CRT, notify Host and Power service
        if state.cur_temp >= utils::dk_to_c(state.uid_table[&uid::CRT_TEMP]) {
            thermal_service::send_service_msg(
                comms::EndpointID::External(comms::External::Host),
                &mptf::Notify::Critical,
            )
            .await;

            thermal_service::send_service_msg(
                comms::EndpointID::Internal(comms::Internal::Power),
                &mptf::Notify::Critical,
            )
            .await;

            warn!("Temperature exceeds CRT!");
        }
    }
}

impl Handler for DefaultHandler {
    async fn get_tmp(&self, _tzid: mptf::TzId) -> mptf::Response {
        Ok(mptf::ResponseData::GetTmp(utils::c_to_dk(
            self.state.lock().await.cur_temp,
        )))
    }

    async fn get_thrs(&self, _tzid: mptf::TzId) -> mptf::Response {
        let low = utils::c_to_dk(self.state.lock().await.threshold_low);
        let high = utils::c_to_dk(self.state.lock().await.threshold_high);
        Ok(mptf::ResponseData::GetThrs(0, low, high))
    }

    async fn set_thrs(
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
        thermal_service::execute_sensor_request(SENSOR, sensor::Request::SetAlertLow(low))
            .await
            .map_err(|_| mptf::Error::HardwareError)?;
        thermal_service::execute_sensor_request(SENSOR, sensor::Request::SetAlertHigh(high))
            .await
            .map_err(|_| mptf::Error::HardwareError)?;

        // Then update state
        self.state.lock().await.threshold_low = low;
        self.state.lock().await.threshold_high = high;
        Ok(mptf::ResponseData::SetThrs)
    }

    async fn set_scp(
        &self,
        _tzid: mptf::TzId,
        _mode: mptf::Dword,
        _acoustic_lim: mptf::Dword,
        _power_lim: mptf::Dword,
    ) -> mptf::Response {
        // Currently default algorithm does not use distinct cooling policies
        Ok(mptf::ResponseData::SetScp)
    }

    async fn get_var(&self, uid: mptf::Uid) -> mptf::Response {
        self.state
            .lock()
            .await
            .uid_table
            .get(&uid)
            .ok_or(mptf::Error::InvalidParameter)
            .map(|&val| mptf::ResponseData::GetVar(uid, val))
    }

    async fn set_var(&self, uid: mptf::Uid, value: mptf::Dword) -> mptf::Response {
        let uid_table = &mut self.state.lock().await.uid_table;

        if let Some(inner_val) = uid_table.get_mut(&uid) {
            *inner_val = value;
            Ok(mptf::ResponseData::SetVar)
        } else {
            Err(mptf::Error::InvalidParameter)
        }
    }
}

/// Default fan control task which interfaces with the default MPTF Handler
#[embassy_executor::task]
pub async fn task(spawner: embassy_executor::Spawner, handler: &'static DefaultHandler) {
    spawner.must_spawn(threshold_task());

    info!("MPTF Default Handler Task Started");
    loop {
        // Fetch and cache most recent temperature from sensor
        handler.state.lock().await.cur_temp =
            match thermal_service::execute_sensor_request(SENSOR, sensor::Request::GetTemp).await {
                Ok(sensor::ResponseData::Temp(temp)) => temp,
                Ok(response) => {
                    warn!("Received unexpected response from sensor: {:?}", response);
                    handler.state.lock().await.cur_temp
                }
                Err(e) => {
                    error!("Error reading temperature from sensor: {:?}", e);
                    handler.state.lock().await.cur_temp
                }
            };

        // Cache most recent fan RPM measurement
        match thermal_service::execute_fan_request(FAN, fan::Request::GetRpm).await {
            fan::Response::Ok(fan::ResponseData::Rpm(rpm)) => {
                if let Err(e) = handler.set_var(uid::FAN_CURRENT_RPM, rpm as mptf::Dword).await {
                    error!("Error caching fan RPM: {:?}", e);
                }
            }
            Ok(response) => warn!("Received unexpected response from fan: {:?}", response),
            Err(e) => error!("Error reading RPM from fan: {:?}", e),
        }

        // Check if the temperature exceeds CRT/PROCHOT and notify services if so
        handler.crt_prochot_check().await;

        // Handle fan state in response to temperature
        if let Err(e) = handler.handle_fan_state().await {
            error!("Error handling fan state: {:?}", e);
        }

        // Wait briefly
        // TODO: Investigate setting sensor alert thresholds to minimum of all thresholds we are interested in as opposed to periodic polling
        Timer::after_millis(1000).await;
    }
}

// Waits for sensor to trigger threshold alert, then notifies host
#[embassy_executor::task]
async fn threshold_task() {
    loop {
        match thermal_service::get_sensor(SENSOR)
            .await
            .expect("Error: Invalid sensor in thermal service")
            .wait_alert()
            .await
        {
            // Notify SoC/Host high threshold was tripped
            // TODO: Better understand notification format
            sensor::Alert::ThresholdLow | sensor::Alert::ThresholdHigh => {
                thermal_service::send_service_msg(
                    comms::EndpointID::External(comms::External::Host),
                    &mptf::Notify::Threshold,
                )
                .await
            }
        }
    }
}
