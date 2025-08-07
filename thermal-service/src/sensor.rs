//! Sensor Device
use crate::utils::SampleBuf;
use crate::{Event, send_event};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdSet};
use embedded_services::GlobalRawMutex;
use embedded_services::error;
use embedded_services::ipc::deferred as ipc;
use embedded_services::{Node, intrusive_list};

// Timeout period (in ms) for physical bus access
const BUS_TIMEOUT: u64 = 200;

/// Convenience type for Sensor response result
pub type Response = Result<ResponseData, Error>;

/// Allows OEM to implement custom requests
///
/// The default response is to return an error on unrecognized requests
pub trait CustomRequestHandler {
    fn handle_custom_request(&self, _request: Request) -> impl core::future::Future<Output = Response> {
        async { Err(Error::InvalidRequest) }
    }
}

/// Ensures all necessary traits are implemented for the controlling driver
pub trait Controller: TemperatureSensor + TemperatureThresholdSet + CustomRequestHandler {}

/* Helper macro for calling a bus function with automatic retry after timeout or failure.
 *
 * Necessary since often the sensor bus is shared and occasionally the underlying bus driver
 * gets in a bad state and can hang or report spurious errors.
 */
macro_rules! with_retry {
    (
        $self:expr,
        $bus_method:expr
    ) => {{
        let mut retry_attempts = $self.profile.lock().await.retry_attempts;

        loop {
            if retry_attempts == 0 {
                break Err(Error::Hardware);
            }

            match embassy_time::with_timeout(embassy_time::Duration::from_millis(BUS_TIMEOUT), $bus_method).await {
                Ok(Ok(val)) => break Ok(val),
                _ => {
                    retry_attempts -= 1;
                }
            }
        }
    }};
}

/// Sensor threshold type
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThresholdType {
    /// Threshold below which host is notified
    WarnLow,
    /// Threshold above which host is notified
    WarnHigh,
    /// Threshold above which PROCHOT is asserted
    Prochot,
    /// Threshold above which critical temperature is reached and system should be shutdown
    /// Some systems may tie sensor alert pin directly to reset controller, in which case
    /// SetHardAlert should be used.
    Critical,
}

/// Sensor error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Invalid request
    InvalidRequest,
    /// Device encountered a hardware failure
    Hardware,
}

/// Sensor request
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    /// Most recent cached temperature measurement
    GetTemp,
    /// Average temperature measurement (over BUFFER_SIZE * SAMPLING_PERIOD)
    GetAvgTemp,
    /// Instructs sensor to immediately sample temperature (not cached)
    GetTmpNow,
    /// Low threshold below which sensor will set the alert pin active (in degrees Celsius)
    SetHardAlertLow(DegreesCelsius),
    /// High threshold above which sensor will set the alert pin active (in degrees Celsius)
    SetHardAlertHigh(DegreesCelsius),
    /// Get a threshold
    GetThreshold(ThresholdType),
    /// Set a threshold
    SetThreshold(ThresholdType, DegreesCelsius),
    /// Threshold in which sensor begins fast sampling
    SetFastSamplingThreshold(DegreesCelsius),
    /// Set temperature sampling period (in ms)
    SetSamplingPeriod(u64),
    /// Set fast temperature sampling period (in ms)
    SetFastSamplingPeriod(u64),
    /// An offset that is applied to all physical temperature samples (in degrees Celsius)
    SetOffset(DegreesCelsius),
    /// Enable sensor sampling
    EnableSampling,
    /// Disable sensor sampling
    DisableSampling,
    /// Set the max number of times communication with physical sensor will be attempted until error is reported
    SetRetryAttempts(u8),
    /// Get the thermal profile associated with this sensor
    GetProfile,
    /// Set the thermal profile associated with this sensor
    SetProfile(Profile),
    /// Custom-implemented command
    Custom(u8, &'static [u8]),
}

/// Sensor response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// Response for any request that is successful but does not require data
    Success,
    /// Temperature (in degrees Celsius)
    Temp(DegreesCelsius),
    /// Threshold (in degrees Celsius)
    Threshold(DegreesCelsius),
    /// Profile
    Profile(Profile),
    /// Custom-implemented response
    Custom(&'static [u8]),
}

/// Sensor device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Sensor device struct
pub struct Device {
    /// Intrusive list node allowing Device to be contained in a list
    node: Node,
    /// Device ID
    id: DeviceId,
    /// Channel for IPC requests and responses
    ipc: ipc::Channel<GlobalRawMutex, Request, Response>,
    /// Signal for enable
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

/// Sensor profile
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Profile {
    /// Profile ID
    pub id: usize,
    /// Period (in ms) sensor will sample its temperature
    pub sample_period: u64,
    /// Period (in ms) sensor will sample its temperature when in fast sampling state
    pub fast_sample_period: u64,
    /// Whether or not automatic background sampling is enabled or not
    pub sampling_enabled: bool,
    /// Hysteresis value (in degrees Celsius) preventing sensor from rapidly reporting threshold events
    pub hysteresis: DegreesCelsius,
    /// Threshold (in degrees Celsius) at which sensor will trigger a WARN LOW event
    pub warn_low_threshold: DegreesCelsius,
    /// Threshold (in degrees Celsius) at which sensor will trigger a WARN HIGH event
    pub warn_high_threshold: DegreesCelsius,
    /// Threshold (in degrees Celsius) at which sensor will trigger a PROCHOT event
    pub prochot_threshold: DegreesCelsius,
    /// Threshold (in degrees Celsius) at which sensor will trigger a CRITICAL event
    pub crt_threshold: DegreesCelsius,
    /// Threshold (in degrees Celsius) at which sensor will enter the fast sampling state
    pub fast_sampling_threshold: DegreesCelsius,
    /// Offset (in degrees Celsius) to be added to sampled temperature
    pub offset: DegreesCelsius,
    /// Number of attempts sensor will make to communicate with the physical device over the bus
    pub retry_attempts: u8,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: 0,
            sample_period: 1000,
            fast_sample_period: 200,
            sampling_enabled: true,
            warn_low_threshold: DegreesCelsius::MIN,
            warn_high_threshold: DegreesCelsius::MAX,
            prochot_threshold: DegreesCelsius::MAX,
            crt_threshold: DegreesCelsius::MAX,
            fast_sampling_threshold: DegreesCelsius::MAX,
            offset: 0.0,
            retry_attempts: 5,
            hysteresis: 2.0,
        }
    }
}

// Additional Sensor state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct State {
    is_warn_low: bool,
    is_warn_high: bool,
    is_prochot: bool,
    is_critical: bool,
}

/// Wrapper binding a communication device, hardware driver, and additional state.
pub struct Sensor<T: Controller, const SAMPLE_BUF_LEN: usize> {
    /// Sensor communication device
    device: Device,
    /// Sensor controller
    controller: Mutex<GlobalRawMutex, T>,
    /// Sensor profile
    profile: Mutex<GlobalRawMutex, Profile>,
    /// Sensor state
    state: Mutex<GlobalRawMutex, State>,
    /// Cached temperature samples
    samples: Mutex<GlobalRawMutex, SampleBuf<DegreesCelsius, SAMPLE_BUF_LEN>>,
}

impl<T: Controller, const SAMPLE_BUF_LEN: usize> Sensor<T, SAMPLE_BUF_LEN> {
    /// New sensor
    ///
    /// Sample buffer length MUST be a power of two
    pub fn new(id: DeviceId, controller: T, profile: Profile) -> Self {
        Self {
            device: Device::new(id),
            controller: Mutex::new(controller),
            profile: Mutex::new(profile),
            state: Mutex::new(State::default()),
            samples: Mutex::new(SampleBuf::create()),
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

    /// Wait for sensor to receive a request
    pub async fn wait_request(&self) -> ipc::Request<'_, GlobalRawMutex, Request, Response> {
        self.device.ipc.receive().await
    }

    /// Process sensor request
    pub async fn process_request(&self, request: Request) -> Response {
        match request {
            Request::GetTemp => {
                let temp = self.samples.lock().await.recent();
                Ok(ResponseData::Temp(temp))
            }
            Request::GetAvgTemp => {
                let temp = self.samples.lock().await.average();
                Ok(ResponseData::Temp(temp))
            }
            Request::GetTmpNow => {
                let temp = with_retry!(self, self.controller.lock().await.temperature())?;
                Ok(ResponseData::Temp(temp))
            }
            Request::SetHardAlertLow(low) => {
                with_retry!(self, self.controller.lock().await.set_temperature_threshold_low(low))?;
                Ok(ResponseData::Success)
            }
            Request::SetHardAlertHigh(high) => {
                with_retry!(self, self.controller.lock().await.set_temperature_threshold_high(high))?;
                Ok(ResponseData::Success)
            }
            Request::GetThreshold(ThresholdType::WarnLow) => {
                let threshold = self.profile.lock().await.warn_low_threshold;
                Ok(ResponseData::Threshold(threshold))
            }
            Request::GetThreshold(ThresholdType::WarnHigh) => {
                let threshold = self.profile.lock().await.warn_high_threshold;
                Ok(ResponseData::Threshold(threshold))
            }
            Request::GetThreshold(ThresholdType::Prochot) => {
                let threshold = self.profile.lock().await.prochot_threshold;
                Ok(ResponseData::Threshold(threshold))
            }
            Request::GetThreshold(ThresholdType::Critical) => {
                let threshold = self.profile.lock().await.crt_threshold;
                Ok(ResponseData::Threshold(threshold))
            }
            Request::SetThreshold(ThresholdType::WarnLow, threshold) => {
                self.profile.lock().await.warn_low_threshold = threshold;
                Ok(ResponseData::Success)
            }
            Request::SetThreshold(ThresholdType::WarnHigh, threshold) => {
                self.profile.lock().await.warn_high_threshold = threshold;
                Ok(ResponseData::Success)
            }
            Request::SetThreshold(ThresholdType::Prochot, threshold) => {
                self.profile.lock().await.prochot_threshold = threshold;
                Ok(ResponseData::Success)
            }
            Request::SetThreshold(ThresholdType::Critical, threshold) => {
                self.profile.lock().await.crt_threshold = threshold;
                Ok(ResponseData::Success)
            }
            Request::SetFastSamplingThreshold(threshold) => {
                self.profile.lock().await.fast_sampling_threshold = threshold;
                Ok(ResponseData::Success)
            }
            Request::SetSamplingPeriod(period) => {
                self.profile.lock().await.sample_period = period;
                Ok(ResponseData::Success)
            }
            Request::SetFastSamplingPeriod(period) => {
                self.profile.lock().await.fast_sample_period = period;
                Ok(ResponseData::Success)
            }
            Request::SetOffset(offset) => {
                self.profile.lock().await.offset = offset;
                Ok(ResponseData::Success)
            }
            Request::EnableSampling => {
                self.profile.lock().await.sampling_enabled = true;
                self.device.enable.signal(());
                Ok(ResponseData::Success)
            }
            Request::DisableSampling => {
                self.profile.lock().await.sampling_enabled = false;
                Ok(ResponseData::Success)
            }
            Request::SetRetryAttempts(limit) => {
                self.profile.lock().await.retry_attempts = limit;
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

    // Wait for sensor to receive a request, process it, and send a response
    pub async fn wait_and_process(&self) {
        let request = self.wait_request().await;
        let response = self.process_request(request.command).await;
        request.respond(response);
    }

    /// Waits for a request then processes it and sends a response
    pub async fn handle_rx(&self) {
        loop {
            self.wait_and_process().await;
        }
    }

    async fn check_thresholds(&self, temp: DegreesCelsius) {
        let profile = self.profile.lock().await;
        let mut state = self.state.lock().await;

        if temp >= profile.warn_high_threshold && !state.is_warn_high {
            send_event(Event::ThresholdExceeded(self.device.id, ThresholdType::WarnHigh, temp)).await;
            state.is_warn_high = true;
        } else if temp < (profile.warn_high_threshold - profile.hysteresis) && state.is_warn_high {
            send_event(Event::ThresholdCleared(self.device.id, ThresholdType::WarnHigh)).await;
            state.is_warn_high = false;
        }

        if temp <= profile.warn_low_threshold && !state.is_warn_low {
            send_event(Event::ThresholdExceeded(self.device.id, ThresholdType::WarnLow, temp)).await;
            state.is_warn_low = true;
        } else if temp > (profile.warn_low_threshold + profile.hysteresis) && state.is_warn_low {
            send_event(Event::ThresholdCleared(self.device.id, ThresholdType::WarnLow)).await;
            state.is_warn_low = false;
        }

        if temp >= profile.prochot_threshold && !state.is_prochot {
            send_event(Event::ThresholdExceeded(self.device.id, ThresholdType::Prochot, temp)).await;
            state.is_prochot = true;
        } else if temp < (profile.prochot_threshold - profile.hysteresis) && state.is_prochot {
            send_event(Event::ThresholdCleared(self.device.id, ThresholdType::Prochot)).await;
            state.is_prochot = false;
        }

        if temp >= profile.crt_threshold && !state.is_critical {
            send_event(Event::ThresholdExceeded(self.device.id, ThresholdType::Critical, temp)).await;
            state.is_critical = true;
        } else if temp < (profile.crt_threshold - profile.hysteresis) && state.is_critical {
            send_event(Event::ThresholdCleared(self.device.id, ThresholdType::Critical)).await;
            state.is_critical = false;
        }
    }

    /// Periodically samples temperature from physical sensor and caches it
    pub async fn handle_sampling(&self) {
        loop {
            // Only sample temperature if enabled
            if self.profile.lock().await.sampling_enabled {
                let temp = match with_retry!(self, self.controller.lock().await.temperature()) {
                    Ok(temp) => temp,
                    _ => {
                        self.profile.lock().await.sampling_enabled = false;
                        send_event(Event::SensorFailure(self.device.id, Error::Hardware)).await;
                        error!("Error sampling sensor {}, disabling sampling", self.device.id.0);
                        continue;
                    }
                };

                // Add offset to measured temperature
                let temp = temp + self.profile.lock().await.offset;

                // Cache in buffer for quick retrieval from other services
                self.samples.lock().await.push(temp);

                // Check thresholds
                self.check_thresholds(temp).await;

                // Adjust sampling rate based on how hot we are getting
                let profile = self.profile.lock().await;
                let sleep_duration = if temp >= profile.fast_sampling_threshold {
                    profile.fast_sample_period
                } else {
                    profile.sample_period
                };
                drop(profile);

                // Sleep in-between sampling periods
                Timer::after_millis(sleep_duration).await;

            // Otherwise sleep and wait to be re-enabled
            } else {
                self.device.enable.wait().await;
            }
        }
    }
}

/// This is a public helper macro for implementing the sensor task since tasks cannot be generic
#[macro_export]
macro_rules! impl_sensor_task {
    ($sensor_task_name:ident, $sensor_type:ty, $sample_buf_len:expr) => {
        #[embassy_executor::task]
        pub async fn $sensor_task_name(sensor: &'static $crate::sensor::Sensor<$sensor_type, $sample_buf_len>) {
            let _ = embassy_futures::join::join(sensor.handle_rx(), sensor.handle_sampling()).await;
        }
    };
}
