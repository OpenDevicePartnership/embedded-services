use crate::fan;
use crate::mptf;
use crate::sensor;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_services::comms;
use embedded_services::error;
use embedded_services::info;

/// Convert deciKelvin to degrees Celsius
pub const fn dk_to_c(dk: mptf::DeciKelvin) -> f32 {
    (dk / 10) as f32 - 273.15
}

/// Convert degrees Celsius to deciKelvin
pub const fn c_to_dk(c: f32) -> mptf::DeciKelvin {
    ((c + 273.15) * 10.0) as mptf::DeciKelvin
}

#[allow(async_fn_in_trait)]
pub trait ThermalZone {
    async fn get_tmp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_thrs(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_thrs(
        &self,
        timeout: mptf::Dword,
        low: mptf::Dword,
        high: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error>;

    async fn set_scp(
        &self,
        cooling_policy: mptf::Dword,
        acoustic_lim: mptf::Dword,
        power_lim: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error>;

    async fn get_crt_temp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_crt_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error>;

    async fn get_proc_hot_temp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_proc_hot_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error>;

    async fn get_profile_type(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_profile_type(&self, profile_type: mptf::Dword) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_on_temp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_fan_on_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_ramp_temp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_fan_ramp_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_max_temp(&self) -> Result<mptf::Response, mptf::Error>;

    async fn set_fan_max_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_min_rpm(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_max_rpm(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_current_rpm(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_min_dba(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_max_dba(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_current_dba(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_min_sones(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_max_sones(&self) -> Result<mptf::Response, mptf::Error>;

    async fn get_fan_current_sones(&self) -> Result<mptf::Response, mptf::Error>;

    async fn ramp_response(&self, temp: f32) -> Result<(), ()>;
}

impl<T: ThermalZone + ?Sized> ThermalZone for &T {
    async fn get_tmp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_tmp(self).await
    }

    async fn get_thrs(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_thrs(self).await
    }

    async fn set_thrs(
        &self,
        timeout: mptf::Dword,
        low: mptf::Dword,
        high: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error> {
        T::set_thrs(self, timeout, low, high).await
    }

    async fn set_scp(
        &self,
        cooling_policy: mptf::Dword,
        acoustic_lim: mptf::Dword,
        power_lim: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error> {
        T::set_scp(self, cooling_policy, acoustic_lim, power_lim).await
    }

    async fn get_crt_temp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_crt_temp(self).await
    }

    async fn set_crt_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        T::set_crt_temp(self, temp).await
    }

    async fn get_proc_hot_temp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_proc_hot_temp(self).await
    }

    async fn set_proc_hot_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        T::set_proc_hot_temp(self, temp).await
    }

    async fn get_profile_type(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_profile_type(self).await
    }

    async fn set_profile_type(&self, profile_type: mptf::Dword) -> Result<mptf::Response, mptf::Error> {
        T::set_profile_type(self, profile_type).await
    }

    async fn get_fan_on_temp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_on_temp(self).await
    }

    async fn set_fan_on_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        T::set_fan_on_temp(self, temp).await
    }

    async fn get_fan_ramp_temp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_ramp_temp(self).await
    }

    async fn set_fan_ramp_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        T::set_fan_ramp_temp(self, temp).await
    }

    async fn get_fan_max_temp(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_max_temp(self).await
    }

    async fn set_fan_max_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        T::set_fan_max_temp(self, temp).await
    }

    async fn get_fan_min_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_min_rpm(self).await
    }

    async fn get_fan_max_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_max_rpm(self).await
    }

    async fn get_fan_current_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_current_rpm(self).await
    }

    async fn get_fan_min_dba(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_min_dba(self).await
    }

    async fn get_fan_max_dba(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_max_dba(self).await
    }

    async fn get_fan_current_dba(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_current_dba(self).await
    }

    async fn get_fan_min_sones(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_min_sones(self).await
    }

    async fn get_fan_max_sones(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_max_sones(self).await
    }

    async fn get_fan_current_sones(&self) -> Result<mptf::Response, mptf::Error> {
        T::get_fan_current_sones(self).await
    }

    async fn ramp_response(&self, temp: f32) -> Result<(), ()> {
        T::ramp_response(self, temp).await
    }
}

enum FanState {
    Off,
    On,
    Ramping,
    Max,
}

// State for the generic MPTF thermal zone
struct GenericThermalZoneState {
    cur_temp: f32, // Cached, previous measured temperature

    thresholds: (u32, f32, f32),
    cooling_policy: u32,
    crt_temp: f32,
    proc_hot_temp: f32,
    profile_type: u32,
    acoustic_lim: u32,
    power_lim: u32,

    fan_on_temp: f32,
    fan_ramp_temp: f32,
    fan_max_temp: f32,
    fan_state: FanState,
}

impl Default for GenericThermalZoneState {
    fn default() -> Self {
        Self {
            cur_temp: 0.0,

            thresholds: (0, 0.0, 0.0),
            cooling_policy: 0,
            crt_temp: 100.0,
            proc_hot_temp: 85.0,
            profile_type: 0,
            acoustic_lim: 0,
            power_lim: 0,

            fan_on_temp: 177.0,
            fan_ramp_temp: 180.0,
            fan_max_temp: 200.0,
            fan_state: FanState::Off,
        }
    }
}

// A generic MPTF Thermal Zone
pub struct GenericThermalZone {
    state: Mutex<NoopRawMutex, GenericThermalZoneState>,
    sensor: &'static sensor::Device,
    fan: &'static fan::Device,
}

impl GenericThermalZone {
    async fn threshold_check(&self, thermal_service: &'static crate::ThermalService<&'static GenericThermalZone>) {
        let state = self.state.lock().await;

        // If temp trips a threshold, notify host (which should notify OSPM)
        if state.cur_temp <= state.thresholds.1 || state.cur_temp >= state.thresholds.2 {
            thermal_service
                .endpoint
                .send(
                    comms::EndpointID::External(comms::External::Host),
                    &mptf::Notify::Threshold,
                )
                .await
                .unwrap();
        }

        // If temp rises above PROCHOT, send the notification somewhere
        if state.cur_temp <= state.thresholds.1 || state.cur_temp >= state.thresholds.2 {
            // TODO: Send PROCHOT notification somewhere
        }

        // If temp rises above critical, notify Host and also notify power service to shutdown
        if state.cur_temp >= state.crt_temp {
            thermal_service
                .endpoint
                .send(
                    comms::EndpointID::External(comms::External::Host),
                    &mptf::Notify::Critical,
                )
                .await
                .unwrap();

            // TODO: Actually figure out message to send to Power service
            thermal_service
                .endpoint
                .send(
                    comms::EndpointID::Internal(comms::Internal::Power),
                    &mptf::Notify::Critical,
                )
                .await
                .unwrap();
        }
    }

    async fn handle_fan_state(&self) {
        let mut state = self.state.lock().await;

        // Handle fan response to measured temperature
        match state.fan_state {
            FanState::Off => {
                // If temp rises above Fan Min On Temp, set fan to min RPM
                if state.cur_temp >= state.fan_on_temp {
                    let min_rpm = match self.fan.execute_request(fan::Request::GetMinRpm).await {
                        Ok(fan::Response::GetMinRpm(rpm)) => rpm,
                        _ => todo!(),
                    };

                    self.fan.execute_request(fan::Request::SetRpm(min_rpm)).await.unwrap();
                    state.fan_state = FanState::On;
                    info!("\n\nFan turned ON\n\n");
                }
            }

            FanState::On => {
                // If temp rises above Fan Ramp Temp, set fan to begin ramp curve
                if state.cur_temp >= state.fan_ramp_temp {
                    state.fan_state = FanState::Ramping;
                    info!("\n\nFan ramping!\n\n");

                // If falls below on temp, turn fan off
                } else if state.cur_temp < state.fan_on_temp {
                    self.fan.execute_request(fan::Request::SetRpm(0)).await.unwrap();
                    state.fan_state = FanState::Off;
                    info!("\n\nFan turned OFF\n\n");
                }
            }

            FanState::Ramping => {
                // If temp falls below ramp temp, set to On state
                if state.cur_temp < state.fan_ramp_temp {
                    let min_rpm = match self.fan.execute_request(fan::Request::GetMinRpm).await {
                        Ok(fan::Response::GetMinRpm(rpm)) => rpm,
                        _ => todo!(),
                    };

                    self.fan.execute_request(fan::Request::SetRpm(min_rpm)).await.unwrap();
                    state.fan_state = FanState::On;
                }

                // If temp stays below max temp, continue ramp response
                if state.cur_temp < state.fan_max_temp {
                    self.ramp_response(state.cur_temp).await.unwrap();

                // If above max, go to max state
                } else {
                    let max_rpm = match self.fan.execute_request(fan::Request::GetMaxRpm).await {
                        Ok(fan::Response::GetMaxRpm(rpm)) => rpm,
                        _ => todo!(),
                    };

                    self.fan.execute_request(fan::Request::SetRpm(max_rpm)).await.unwrap();
                    state.fan_state = FanState::Max;
                    info!("\n\nFan at MAX!\n\n");
                }
            }

            FanState::Max => {
                if state.cur_temp < state.fan_max_temp {
                    state.fan_state = FanState::Ramping;
                }
            }
        }
    }

    pub fn new(sensor: &'static sensor::Device, fan: &'static fan::Device) -> Self {
        Self {
            state: Mutex::new(GenericThermalZoneState::default()),
            sensor,
            fan,
        }
    }
}

impl ThermalZone for GenericThermalZone {
    async fn get_tmp(&self) -> Result<mptf::Response, mptf::Error> {
        Ok(mptf::Response::GetTmp(c_to_dk(self.state.lock().await.cur_temp)))
    }

    async fn get_thrs(&self) -> Result<mptf::Response, mptf::Error> {
        let (timeout, low, high) = self.state.lock().await.thresholds;
        let low = c_to_dk(low);
        let high = c_to_dk(high);
        Ok(mptf::Response::GetThrs(timeout, low, high))
    }

    async fn set_thrs(
        &self,
        timeout: mptf::Dword,
        low: mptf::Dword,
        high: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error> {
        let low = dk_to_c(low);
        let high = dk_to_c(high);
        self.state.lock().await.thresholds = (timeout, low, high);
        Ok(mptf::Response::SetThrs)
    }

    async fn set_scp(
        &self,
        cooling_policy: mptf::Dword,
        acoustic_lim: mptf::Dword,
        power_lim: mptf::Dword,
    ) -> Result<mptf::Response, mptf::Error> {
        let mut state = self.state.lock().await;
        state.cooling_policy = cooling_policy;
        state.acoustic_lim = acoustic_lim;
        state.power_lim = power_lim;

        Ok(mptf::Response::SetScp)
    }

    async fn get_crt_temp(&self) -> Result<mptf::Response, mptf::Error> {
        let dk = c_to_dk(self.state.lock().await.crt_temp);
        Ok(mptf::Response::GetCrtTemp(dk))
    }

    async fn set_crt_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.crt_temp = dk_to_c(temp);
        Ok(mptf::Response::SetCrtTemp)
    }

    async fn get_proc_hot_temp(&self) -> Result<mptf::Response, mptf::Error> {
        let dk = c_to_dk(self.state.lock().await.proc_hot_temp);
        Ok(mptf::Response::GetProcHotTemp(dk))
    }

    async fn set_proc_hot_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.proc_hot_temp = dk_to_c(temp);
        Ok(mptf::Response::SetProcHotTemp)
    }

    async fn get_profile_type(&self) -> Result<mptf::Response, mptf::Error> {
        let prof = self.state.lock().await.profile_type;
        Ok(mptf::Response::GetProfileType(prof))
    }

    async fn set_profile_type(&self, profile_type: mptf::Dword) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.profile_type = profile_type;
        Ok(mptf::Response::SetProfileType)
    }

    async fn get_fan_on_temp(&self) -> Result<mptf::Response, mptf::Error> {
        let dk = c_to_dk(self.state.lock().await.fan_on_temp);
        Ok(mptf::Response::GetFanOnTemp(dk))
    }

    async fn set_fan_on_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.fan_on_temp = dk_to_c(temp);
        Ok(mptf::Response::SetFanOnTemp)
    }

    async fn get_fan_ramp_temp(&self) -> Result<mptf::Response, mptf::Error> {
        let dk = c_to_dk(self.state.lock().await.fan_ramp_temp);
        Ok(mptf::Response::GetFanRampTemp(dk))
    }

    async fn set_fan_ramp_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.fan_ramp_temp = dk_to_c(temp);
        Ok(mptf::Response::SetFanRampTemp)
    }

    async fn get_fan_max_temp(&self) -> Result<mptf::Response, mptf::Error> {
        let dk = c_to_dk(self.state.lock().await.fan_max_temp);
        Ok(mptf::Response::GetFanMaxTemp(dk))
    }

    async fn set_fan_max_temp(&self, temp: mptf::DeciKelvin) -> Result<mptf::Response, mptf::Error> {
        self.state.lock().await.fan_max_temp = dk_to_c(temp);
        Ok(mptf::Response::SetFanMaxTemp)
    }

    async fn get_fan_min_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMinRpm).await {
            Ok(fan::Response::GetMinRpm(rpm)) => Ok(mptf::Response::GetFanMinRpm(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_max_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMaxRpm).await {
            Ok(fan::Response::GetMinRpm(rpm)) => Ok(mptf::Response::GetFanMaxRpm(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_current_rpm(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetRpm).await {
            Ok(fan::Response::GetRpm(rpm)) => Ok(mptf::Response::GetFanCurrentRpm(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_min_dba(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMinDba).await {
            Ok(fan::Response::GetMinDba(rpm)) => Ok(mptf::Response::GetFanMinDba(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_max_dba(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMinDba).await {
            Ok(fan::Response::GetMinDba(rpm)) => Ok(mptf::Response::GetFanMinDba(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_current_dba(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetDba).await {
            Ok(fan::Response::GetDba(rpm)) => Ok(mptf::Response::GetFanCurrentDba(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_min_sones(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMinSones).await {
            Ok(fan::Response::GetMinSones(rpm)) => Ok(mptf::Response::GetFanMinSones(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_max_sones(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetMaxSones).await {
            Ok(fan::Response::GetMaxSones(rpm)) => Ok(mptf::Response::GetFanMaxSones(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn get_fan_current_sones(&self) -> Result<mptf::Response, mptf::Error> {
        match self.fan.execute_request(fan::Request::GetSones).await {
            Ok(fan::Response::GetSones(rpm)) => Ok(mptf::Response::GetFanCurrentSones(rpm)),
            _ => Err(mptf::Error::HardwareError),
        }
    }

    async fn ramp_response(&self, temp: f32) -> Result<(), ()> {
        let min_rpm = match self.fan.execute_request(fan::Request::GetMinRpm).await {
            Ok(fan::Response::GetMinRpm(rpm)) => rpm,
            _ => return Err(()),
        };
        let max_rpm = match self.fan.execute_request(fan::Request::GetMaxRpm).await {
            Ok(fan::Response::GetMaxRpm(rpm)) => rpm,
            _ => return Err(()),
        };

        // Some response curve that makes no sense at all for now
        let set_rpm = (max_rpm - min_rpm) / temp as u32;
        self.fan.execute_request(fan::Request::SetRpm(set_rpm)).await.unwrap();

        Ok(())
    }
}

// TODO: Make this actually generic over any impl ThermalZone, so OEM can use this glue logic
#[embassy_executor::task]
pub async fn generic_task(
    thermal_service: &'static crate::ThermalService<&'static GenericThermalZone>,
    tz: &'static GenericThermalZone,
) {
    // Proof of concept logic
    // Would be rewritten to make better use of threshold alert interrupts as opposed to time based polling
    loop {
        // Measure current temperature
        tz.state.lock().await.cur_temp = match tz.sensor.execute_request(sensor::Request::GetCurTemp).await {
            Ok(sensor::Response::GetCurTemp(temp)) => temp,
            Err(e) => {
                error!("Error reading temperature: {:?}", e);
                tz.state.lock().await.cur_temp
            }
            _ => {
                error!("Unknown error occurred.");
                tz.state.lock().await.cur_temp
            }
        };

        // Check if the current temperature exceeds various thresholds and act accordingly
        tz.threshold_check(thermal_service).await;

        // Handle fan state in response to current temperature
        tz.handle_fan_state().await;

        // Wait briefly
        Timer::after_millis(1000).await;
    }
}
