//! Fan Device
use crate::utils::SampleBuf;
use crate::{Event, send_event};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_fans_async::{self as fan_traits, Error as HardwareError};
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::GlobalRawMutex;
use embedded_services::ipc::deferred as ipc;
use embedded_services::{Node, intrusive_list};
use embedded_services::{error, trace};

/// Convenience type for Fan response result
pub type Response = Result<ResponseData, Error>;

/// Allows OEM to implement custom requests
///
/// The default response is to return an error on unrecognized requests
pub trait CustomRequestHandler {
    fn handle_custom_request(&self, _request: Request) -> impl core::future::Future<Output = Response> {
        async { Err(Error::InvalidRequest) }
    }
}

/// Allows OEMs to override the default linear response ramp response of fan
pub trait RampResponseHandler: fan_traits::Fan + fan_traits::RpmSense {
    fn handle_ramp_response(
        &mut self,
        profile: &Profile,
        temp: DegreesCelsius,
    ) -> impl core::future::Future<Output = Result<(), Self::Error>> {
        let fan_ramp_temp = profile.ramp_temp;
        let fan_max_temp = profile.max_temp;
        let min_rpm = self.min_start_rpm();
        let max_rpm = self.max_rpm();

        // Provide a linear fan response between its min and max RPM relative to temperature between ramp start and max temp
        let rpm = if temp <= fan_ramp_temp {
            min_rpm
        } else if temp >= fan_max_temp {
            max_rpm
        } else {
            let ratio = (temp - fan_ramp_temp) / (fan_max_temp - fan_ramp_temp);
            let range = (max_rpm - min_rpm) as f32;
            min_rpm + (ratio * range) as u16
        };

        async move {
            self.set_speed_rpm(rpm).await?;
            Ok(())
        }
    }
}

/// Ensures all necessary traits are implemented for the controlling driver
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
    /// Set RPM manually and disable temperature-based control
    SetRpm(u16),
    /// Set duty cycle manually (in percent) and disable temperature-based control
    SetDuty(u8),
    /// Stop the fan and disable temperature-based control
    Stop,
    /// Enable temperature-based control
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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum FanState {
    Off,
    On,
    Ramping,
    Max,
}

/// Fan device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Fan device struct
pub struct Device {
    // Intrusive list node allowing Device to be contained in a list
    node: Node,
    // Device ID
    id: DeviceId,
    // Channel for IPC requests and responses
    ipc: ipc::Channel<GlobalRawMutex, Request, Response>,
    // Signal for auto-control enable
    auto_control_enable: Signal<GlobalRawMutex, ()>,
}

impl Device {
    /// Create a new fan device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: Node::uninit(),
            id,
            ipc: ipc::Channel::new(),
            auto_control_enable: Signal::new(),
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

/// Fan profile
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Profile {
    /// Profile ID
    pub id: usize,
    /// ID of sensor this fan will query for auto control
    pub sensor_id: crate::sensor::DeviceId,
    /// Period (in ms) fan will sample its RPM
    pub sample_period: u64,
    /// Period (in ms) fan will update its state during auto control
    pub update_period: u64,
    /// Whether fan is under automatic temperature-based control or not
    pub auto_control: bool,
    /// Hysteresis value (in degrees Celsius) preventing fan from rapidly switching between states
    pub hysteresis: DegreesCelsius,
    /// Temperature (in degrees Celsius) at which fan will turn on
    pub on_temp: DegreesCelsius,
    /// Temperature (in degrees Celsius) at which fan will begin its ramp response
    pub ramp_temp: DegreesCelsius,
    /// Temperature (in degrees Celsius) at which fan will run at its max speed
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
pub struct Fan<T: Controller, const SAMPLE_BUF_LEN: usize> {
    // Underlying device
    device: Device,
    // Underlying controller
    controller: Mutex<GlobalRawMutex, T>,
    // Fan profile
    profile: Mutex<GlobalRawMutex, Profile>,
    // RPM samples
    samples: Mutex<GlobalRawMutex, SampleBuf<u16, SAMPLE_BUF_LEN>>,
    // State
    state: Mutex<GlobalRawMutex, FanState>,
}

impl<T: Controller, const SAMPLE_BUF_LEN: usize> Fan<T, SAMPLE_BUF_LEN> {
    /// New fan
    ///
    /// Sample buffer length MUST be a power of two
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

    /// Retrieve a Mutex wrapping the underlying controller
    ///
    /// Should only be used to update OEM specific state
    pub fn controller(&self) -> &Mutex<GlobalRawMutex, T> {
        &self.controller
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
                self.change_state(FanState::Off).await?;
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
                // Make sure we actually transition to a known state
                // Next iteration of handle auto control would then put it in actual correct state
                self.change_state(FanState::Off).await?;
                self.profile.lock().await.auto_control = true;
                self.device.auto_control_enable.signal(());
                Ok(ResponseData::Success)
            }
            Request::SetSpeedUpdatePeriod(period) => {
                self.profile.lock().await.update_period = period;
                Ok(ResponseData::Success)
            }
            Request::GetOnTemp => {
                let temp = self.profile.lock().await.on_temp;
                Ok(ResponseData::Temp(temp))
            }
            Request::GetRampTemp => {
                let temp = self.profile.lock().await.ramp_temp;
                Ok(ResponseData::Temp(temp))
            }
            Request::GetMaxTemp => {
                let temp = self.profile.lock().await.max_temp;
                Ok(ResponseData::Temp(temp))
            }
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

    /// Wait for fan to receive a request, process it, and send a response
    pub async fn wait_and_process(&self) {
        let request = self.wait_request().await;
        let response = self.process_request(request.command).await;
        request.respond(response);
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
                Err(e) => error!("Fan {} error sampling fan rpm: {:?}", self.device.id.0, e.kind()),
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
                    _ => {
                        error!(
                            "Fan {} failed to get temperature, disabling auto control and setting speed to max",
                            self.device.id.0
                        );

                        self.profile.lock().await.auto_control = false;
                        if self.controller.lock().await.set_speed_max().await.is_err() {
                            error!("Fan {} failed to set speed to max!", self.device.id.0);
                        }

                        send_event(Event::FanFailure(self.device.id, Error::Hardware)).await;
                        continue;
                    }
                };

                if let Err(e) = self.handle_fan_state(temp).await {
                    send_event(Event::FanFailure(self.device.id, e)).await;
                    error!("Fan {} error handling fan state transition: {:?}", self.device.id.0, e);
                }

                let sleep_duration = self.profile.lock().await.update_period;
                Timer::after_millis(sleep_duration).await;

            // Sleep until auto control is re-enabled
            } else {
                self.device.auto_control_enable.wait().await;
            }
        }
    }

    async fn handle_fan_off_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let profile = self.profile.lock().await;

        if temp >= profile.on_temp {
            self.change_state(FanState::On).await?;
        }

        Ok(())
    }

    async fn handle_fan_on_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let profile = self.profile.lock().await;

        if temp < (profile.on_temp - profile.hysteresis) {
            self.change_state(FanState::Off).await?;
        } else if temp >= profile.ramp_temp {
            self.change_state(FanState::Ramping).await?;
        }

        Ok(())
    }

    async fn handle_fan_ramping_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let profile = self.profile.lock().await;

        if temp < (profile.ramp_temp - profile.hysteresis) {
            self.change_state(FanState::On).await?;
        } else if temp >= profile.max_temp {
            self.change_state(FanState::Max).await?;
        } else {
            self.controller
                .lock()
                .await
                .handle_ramp_response(&profile, temp)
                .await
                .map_err(|_| Error::Hardware)?;
        }

        Ok(())
    }

    async fn handle_fan_max_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        let profile = self.profile.lock().await;

        if temp < (profile.max_temp - profile.hysteresis) {
            self.change_state(FanState::Ramping).await?;
        }

        Ok(())
    }

    async fn change_state(&self, to: FanState) -> Result<(), Error> {
        let mut controller = self.controller.lock().await;
        match to {
            FanState::Off => {
                controller.stop().await.map_err(|_| Error::Hardware)?;
            }
            FanState::On => {
                controller.start().await.map_err(|_| Error::Hardware)?;
            }
            FanState::Ramping => {
                // Ramp state will continuously update RPM according to its ramp response function
            }
            FanState::Max => {
                let max_rpm = controller.max_rpm();
                let _ = controller.set_speed_rpm(max_rpm).await.map_err(|_| Error::Hardware)?;
            }
        }
        drop(controller);

        let state = *self.state.lock().await;
        trace!(
            "Fan {} transitioned to {:?} state from {:?} state",
            self.device.id.0, to, state
        );
        *self.state.lock().await = to;

        Ok(())
    }

    async fn handle_fan_state(&self, temp: DegreesCelsius) -> Result<(), Error> {
        // Must copy state here, if attempt to dereference in match, mutex is still held in match arms
        let state = *self.state.lock().await;
        match state {
            FanState::Off => self.handle_fan_off_state(temp).await,
            FanState::On => self.handle_fan_on_state(temp).await,
            FanState::Ramping => self.handle_fan_ramping_state(temp).await,
            FanState::Max => self.handle_fan_max_state(temp).await,
        }
    }
}

/// This is a public helper macro for wrapping and spawning the various tasks since currently tasks cannot be generic
#[macro_export]
macro_rules! impl_fan_task {
    ($fan_task_name:ident, $fan_type:ty, $sample_buf_len:expr) => {
        #[embassy_executor::task]
        pub async fn $fan_task_name(fan: &'static $crate::fan::Fan<$fan_type, $sample_buf_len>) {
            let _ =
                embassy_futures::join::join3(fan.handle_rx(), fan.handle_sampling(), fan.handle_auto_control()).await;
        }
    };
}
