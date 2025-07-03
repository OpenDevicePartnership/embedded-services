//! Fan Device
use crate::utils::SampleBuf;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_fans_async::{self as fan_traits, Error as HadrwareError};
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::GlobalRawMutex;
use embedded_services::ipc::deferred as ipc;
use embedded_services::{Node, intrusive_list};
use embedded_services::{error, info};

// RPM sample buffer size
const BUFFER_SIZE: usize = 16;

/// Convenience type for Fan response result
pub type Response = Result<ResponseData, Error>;

pub trait CustomRequestHandler {
    fn handle_custom_request(&self, _request: Request) -> impl core::future::Future<Output = Response> {
        async { Err(Error::InvalidRequest) }
    }
}

pub trait RampResponseHandler: fan_traits::Fan + fan_traits::RpmSense {
    fn handle_ramp_response(
        &mut self,
        profile: &Profile,
        temp: DegreesCelsius,
    ) -> impl core::future::Future<Output = Result<(), Self::Error>> {
        let fan_ramp_tamp = profile.ramp_temp;
        let fan_max_tamp = profile.max_temp;
        let min_rpm = self.min_rpm();
        let max_rpm = self.max_rpm();

        // Provide a linear fan response between its min and max RPM relative to temperature between ramp start and max temp
        let rpm = if temp <= fan_ramp_tamp {
            min_rpm
        } else if temp >= fan_max_tamp {
            max_rpm
        } else {
            let ratio = (temp - fan_ramp_tamp) / (fan_max_tamp - fan_ramp_tamp);
            let range = (max_rpm - min_rpm) as f32;
            min_rpm + (ratio * range) as u16
        };

        async move {
            self.set_speed_rpm(rpm).await?;
            Ok(())
        }
    }
}

pub trait Controller: RampResponseHandler + CustomRequestHandler {}

/// Fan error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Invalid request
    InvalidRequest,
    /// Device encountered a hardware failure
    Hardware,
}

/// Fan request
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    /// Most recent RPM measurement
    GetRpm,
    /// Average RPM measurement
    GetAvgRpm,
    /// Get Min RPM
    GetMinRpm,
    /// Get Max RPM
    GetMaxRpm,
    /// Set RPM manually
    /// This will turn off auto temperature-based control
    SetRpm(u16),
    /// Set duty cycle manually (in percent)
    /// This will turn off auto temperature-based control
    SetDuty(u8),
    /// Stops the fan and disables temperature-based control
    Stop,
    /// Enable auto temperature-based control
    EnableAutoControl,
    /// Set RPM sampling period (in ms)
    SetSamplingPeriod(u64),
    /// Set speed update period
    SetSpeedUpdatePeriod(u64),
    /// Get temperature which fan will turn on to minimum RPM (in degrees Celsius)
    GetOnTemp,
    /// Get temperature which fan will begin ramping (in degrees Celsius)
    GetRampTemp,
    /// Get temperature which fan will reach its max RPM (in degrees Celsius)
    GetMaxTemp,
    /// Set temperature which fan will turn on to minimum RPM (in degrees Celsius)
    SetOnTemp(DegreesCelsius),
    /// Set temperature which fan will begin ramping (in degrees Celsius)
    SetRampTemp(DegreesCelsius),
    /// Set temperature which fan will reach its max RPM (in degrees Celsius)
    SetMaxTemp(DegreesCelsius),
    /// Set hysteresis value between fan on and fan off (in degrees Celsius)
    SetHysteresis(DegreesCelsius),
    /// Get the profile associated with this fan
    GetProfile,
    /// Set the profile associated with this fan
    SetProfile(Profile),
    /// Custom-implemented command
    Custom(u8, &'static [u8]),
}

/// Fan response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// Response for any request that is successful but does not require data
    Success,
    /// RPM
    Rpm(u16),
    /// Temperature
    Temp(DegreesCelsius),
    /// Profile
    Profile(Profile),
    /// Custom-implemented response
    Custom(&'static [u8]),
}

#[derive(Debug, Clone, Copy)]
enum FanState {
    Off,
    On,
    Ramping,
    Max,
}

/// Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Fan device struct
pub struct Device {
    /// Intrusive list node allowing Device to be contained in a list
    node: Node,
    /// Device ID
    id: DeviceId,
    /// Channel for IPC requests and responses
    ipc: ipc::Channel<GlobalRawMutex, Request, Response>,
    /// Signal for auto-control enable
    enable: Signal<GlobalRawMutex, ()>,
}

impl Device {
    /// Create a new sensor device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: Node::uninit(),
            id,
            ipc: ipc::Channel::new(),
            enable: Signal::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Execute request and wait for response
    pub async fn execute_request(&self, request: Request) -> Response {
        self.ipc.execute(request).await
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

// Fan profile
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Profile {
    pub id: usize,
    pub sensor_id: crate::sensor::DeviceId,
    pub sample_period: u64,
    pub update_period: u64,
    pub auto_control: bool,
    pub hysteresis: DegreesCelsius,
    pub on_temp: DegreesCelsius,
    pub ramp_temp: DegreesCelsius,
    pub max_temp: DegreesCelsius,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: 0,
            sensor_id: crate::sensor::DeviceId(0),
            sample_period: 1000,
            update_period: 1000,
            auto_control: true,
            hysteresis: 2.0,
            on_temp: 39.0,
            ramp_temp: 40.0,
            max_temp: 44.0,
        }
    }
}

/// Fan struct containing device for comms and driver
pub struct Fan<T: Controller> {
    /// Underlying device
    device: Device,
    /// Underlying controller
    controller: Mutex<GlobalRawMutex, T>,
    /// Fan profile
    profile: Mutex<GlobalRawMutex, Profile>,
    /// RPM samples
    samples: Mutex<GlobalRawMutex, SampleBuf<u16, BUFFER_SIZE>>,
    /// State
    state: Mutex<GlobalRawMutex, FanState>,
}

impl<T: Controller> Fan<T> {
    /// New fan
    pub fn new(id: DeviceId, controller: T, profile: Profile) -> Self {
        Self {
            device: Device::new(id),
            controller: Mutex::new(controller),
            profile: Mutex::new(profile),
            samples: Mutex::new(SampleBuf::create()),
            state: Mutex::new(FanState::Off),
        }
    }

    /// Retrieve a reference to underlying device for registration with services
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Wait for fan to receive a request, process it, and send a response
    pub async fn wait_and_process(&self) {
        let request = self.wait_request().await;
        let response = self.process_request(request.command).await;
        request.respond(response);
    }

    /// Wait for fan to receive a request
    pub async fn wait_request(&self) -> ipc::Request<'_, GlobalRawMutex, Request, Response> {
        self.device.ipc.receive().await
    }

    /// Process fan request
    pub async fn process_request(&self, request: Request) -> Response {
        match request {
            Request::GetRpm => {
                let rpm = self.samples.lock().await.recent();
                Ok(ResponseData::Rpm(rpm))
            }
            Request::GetAvgRpm => {
                let rpm = self.samples.lock().await.average();
                Ok(ResponseData::Rpm(rpm))
            }
            Request::SetRpm(rpm) => {
                self.controller
                    .lock()
                    .await
                    .set_speed_rpm(rpm)
                    .await
                    .map_err(|_| Error::Hardware)?;
                self.profile.lock().await.auto_control = false;
                Ok(ResponseData::Success)
            }
            Request::SetDuty(percent) => {
                self.controller
                    .lock()
                    .await
                    .set_speed_percent(percent)
                    .await
                    .map_err(|_| Error::Hardware)?;
                self.profile.lock().await.auto_control = false;
                Ok(ResponseData::Success)
            }
            Request::Stop => {
                self.controller.lock().await.stop().await.map_err(|_| Error::Hardware)?;
                self.profile.lock().await.auto_control = false;
                Ok(ResponseData::Success)
            }
            Request::GetMinRpm => {
                let min_rpm = self.controller.lock().await.min_rpm();
                Ok(ResponseData::Rpm(min_rpm))
            }
            Request::GetMaxRpm => {
                let max_rpm = self.controller.lock().await.max_rpm();
                Ok(ResponseData::Rpm(max_rpm))
            }
            Request::SetSamplingPeriod(period) => {
                self.profile.lock().await.sample_period = period;
                Ok(ResponseData::Success)
            }
            Request::EnableAutoControl => {
                self.profile.lock().await.auto_control = true;
                self.device.enable.signal(());
                Ok(ResponseData::Success)
            }
            Request::SetSpeedUpdatePeriod(period) => {
                self.profile.lock().await.update_period = period;
                Ok(ResponseData::Success)
            }
            Request::GetOnTemp => Ok(ResponseData::Temp(self.profile.lock().await.on_temp)),
            Request::GetRampTemp => Ok(ResponseData::Temp(self.profile.lock().await.ramp_temp)),
            Request::GetMaxTemp => Ok(ResponseData::Temp(self.profile.lock().await.max_temp)),
            Request::SetOnTemp(temp) => {
                self.profile.lock().await.on_temp = temp;
                Ok(ResponseData::Success)
            }
            Request::SetRampTemp(temp) => {
                self.profile.lock().await.ramp_temp = temp;
                Ok(ResponseData::Success)
            }
            Request::SetMaxTemp(temp) => {
                self.profile.lock().await.max_temp = temp;
                Ok(ResponseData::Success)
            }
            Request::SetHysteresis(temp) => {
                self.profile.lock().await.hysteresis = temp;
                Ok(ResponseData::Success)
            }
            Request::GetProfile => {
                let profile = *self.profile.lock().await;
                Ok(ResponseData::Profile(profile))
            }
            Request::SetProfile(profile) => {
                *self.profile.lock().await = profile;
                Ok(ResponseData::Success)
            }
            Request::Custom(_, _) => self.controller.lock().await.handle_custom_request(request).await,
        }
    }

    /// Waits for a IPC request, then processes it
    pub async fn handle_rx(&self) {
        loop {
            self.wait_and_process().await;
        }
    }

    /// Periodically samples RPM from physical fan and caches it
    pub async fn handle_sampling(&self) {
        loop {
            match self.controller.lock().await.rpm().await {
                Ok(rpm) => self.samples.lock().await.push(rpm),
                Err(e) => error!("Error sampling rpm: {:?}", e.kind()),
            }

            let period = self.profile.lock().await.sample_period;
            Timer::after_millis(period).await;
        }
    }

    pub async fn handle_auto_control(&self) {
        loop {
            if self.profile.lock().await.auto_control {
                let temp = match crate::execute_sensor_request(
                    self.profile.lock().await.sensor_id,
                    crate::sensor::Request::GetTemp,
                )
                .await
                {
                    Ok(crate::sensor::ResponseData::Temp(temp)) => temp,
                    _ => todo!(),
                };

                if let Err(e) = self.handle_fan_state(temp).await {
                    crate::send_event(crate::Event::FanFailure(self.device.id, e)).await;
                    error!("Error handling fan state transition: {:?}", e);
                }

                let sleep_duration = self.profile.lock().await.update_period;
                Timer::after_millis(sleep_duration).await;

            // Sleep until auto control is re-enabled
            } else {
                self.device.enable.wait().await;
            }
        }
    }

    async fn handle_fan_off_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let config = self.profile.lock().await;

        // If temp rises above Fan Min On Temp, set fan to ON state
        if temp >= config.on_temp {
            let min_rpm = self.controller.lock().await.min_rpm();

            let _ = self
                .controller
                .lock()
                .await
                .set_speed_rpm(min_rpm)
                .await
                .map_err(|_| Error::Hardware)?;

            *self.state.lock().await = FanState::On;
            info!("Fan transitioned to ON state from OFF state");
        }

        Ok(())
    }

    async fn handle_fan_on_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let config = self.profile.lock().await;

        // If temp rises above Fan Ramp Temp, set to RAMPING state
        if temp >= config.ramp_temp {
            *self.state.lock().await = FanState::Ramping;
            info!("Fan transitioned to RAMPING state from ON state");

        // If falls below on temp, set to OFF state
        // TODO: Handle hysteresis
        } else if temp < config.on_temp {
            self.controller.lock().await.stop().await.map_err(|_| Error::Hardware)?;

            *self.state.lock().await = FanState::Off;
            info!("Fan transitioned to OFF state from ON state");
        }

        Ok(())
    }

    async fn handle_fan_ramping_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let min_rpm = self.controller.lock().await.min_rpm();
        let max_rpm = self.controller.lock().await.max_rpm();

        // If temp falls below ramp temp, set to ON state
        if temp < self.profile.lock().await.ramp_temp {
            let _ = self
                .controller
                .lock()
                .await
                .set_speed_rpm(min_rpm)
                .await
                .map_err(|_| Error::Hardware)?;

            *self.state.lock().await = FanState::On;
            info!("Fan transitioned to ON state from RAMPING state");

        // If temp rises above max, set to MAX state
        } else if temp >= self.profile.lock().await.max_temp {
            let _ = self
                .controller
                .lock()
                .await
                .set_speed_rpm(max_rpm)
                .await
                .map_err(|_| Error::Hardware)?;

            *self.state.lock().await = FanState::Max;
            info!("Fan transitioned to MAX state from RAMPING state");

        // If temp stays between ramp temp and max temp, continue ramp response
        } else {
            self.controller
                .lock()
                .await
                .handle_ramp_response(&*self.profile.lock().await, temp)
                .await
                .map_err(|_| Error::Hardware)?;
        }

        Ok(())
    }

    async fn handle_fan_max_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let config = self.profile.lock().await;

        if temp < config.max_temp {
            *self.state.lock().await = FanState::Ramping;
            info!("Fan transitioned to RAMPING state from MAX state");
        }

        Ok(())
    }

    async fn handle_fan_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let state = *self.state.lock().await;

        match state {
            FanState::Off => self.handle_fan_off_state(temp).await,
            FanState::On => self.handle_fan_on_state(temp).await,
            FanState::Ramping => self.handle_fan_ramping_state(temp).await,
            FanState::Max => self.handle_fan_max_state(temp).await,
        }
    }
}

/// This is a helper macro for wrapping and spawning the various tasks since currently tasks cannot be generic
#[macro_export]
macro_rules! impl_fan_task {
    ($fan_task_name:ident, $fan_type:ty) => {
        #[embassy_executor::task]
        pub async fn $fan_task_name(fan: &'static $crate::fan::Fan<$fan_type>) {
            embedded_services::info!("Fan task started!");

            let _ = embassy_futures::select::select3(fan.handle_rx(), fan.handle_sampling(), fan.handle_auto_control())
                .await;
        }
    };
}
