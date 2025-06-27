//! Sensor Device
use crate::utils::SampleBuf;
use core::sync::atomic::AtomicBool;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_sensors_hal_async::sensor::Error as HardwareError;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdSet};
use embedded_services::error;
use embedded_services::ipc::deferred as ipc;
use embedded_services::{intrusive_list, Node};

// Temperature sample buffer size
const BUFFER_SIZE: usize = 16;

/// Convenience type for Sensor response result
pub type Response = Result<ResponseData, Error>;

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
pub enum Request {
    /// Most recent temperature measurement
    GetTemp,
    /// Average temperature measurement
    GetAvgTemp,
    /// Set low alert thresholds (in degrees Celsius)
    SetAlertLow(DegreesCelsius),
    /// Set high alert thresholds (in degrees Celsius)
    SetAlertHigh(DegreesCelsius),
    /// Set temperature sampling period (in ms)
    SetSamplingPeriod(u64),
    /// Enable sensor sampling
    Enable,
    /// Disable sensor sampling
    Disable,
}

/// Sensor response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// Response for any request that is successful but does not require data
    Success,
    /// Temperature (in degrees Celsisus)
    Temp(DegreesCelsius),
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Alert {
    /// High threshold was crossed
    ThresholdLow,
    /// Low threshold was crossed
    ThresholdHigh,
}

/// Device ID new type
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
    ipc: ipc::Channel<NoopRawMutex, Request, Response>,
    /// Signal for threshold alerts from this device
    alert: Signal<NoopRawMutex, Alert>,
    /// Signal for enable
    enable: Signal<NoopRawMutex, ()>,
}

impl Device {
    /// Create a new sensor device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: Node::uninit(),
            id,
            ipc: ipc::Channel::new(),
            alert: Signal::new(),
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

    /// Wait for sensor to generate an alert
    pub async fn wait_alert(&self) -> Alert {
        self.alert.wait().await
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

// Internal sensor state
struct State {
    samples: SampleBuf<DegreesCelsius, BUFFER_SIZE>,
    period: u64,
    enabled: AtomicBool,
    alert_low: DegreesCelsius,
    alert_high: DegreesCelsius,
}

impl Default for State {
    fn default() -> Self {
        Self {
            samples: SampleBuf::create(),
            period: 200,
            enabled: AtomicBool::new(true),
            alert_low: DegreesCelsius::MAX,
            alert_high: DegreesCelsius::MAX,
        }
    }
}

/// Wrapper binding a communication device, hardware driver, and additional state.
pub struct Sensor<T: TemperatureSensor + TemperatureThresholdSet> {
    /// Underlying device
    device: Device,
    /// Underlying driver
    driver: Mutex<NoopRawMutex, T>,
    /// Underlying sensor state
    state: Mutex<NoopRawMutex, State>,
}

impl<T: TemperatureSensor + TemperatureThresholdSet> Sensor<T> {
    /// New sensor wrapper
    pub fn new(id: DeviceId, controller: T) -> Self {
        Self {
            device: Device::new(id),
            driver: Mutex::new(controller),
            state: Mutex::new(State::default()),
        }
    }

    /// Retrieve a reference to underlying device for registtation with services
    pub fn device(&self) -> &Device {
        &self.device
    }

    // Enable sensor sampling
    async fn enable(&self) {
        self.state
            .lock()
            .await
            .enabled
            .store(true, core::sync::atomic::Ordering::SeqCst);

        // Signal to wake sensor
        self.device.enable.signal(());
    }

    // Disable sensor sampling
    async fn disable(&self) {
        self.state
            .lock()
            .await
            .enabled
            .store(false, core::sync::atomic::Ordering::SeqCst);
    }

    /// Wait for sensor to receive a request, process it, and send a response
    async fn wait_and_process(&self) {
        let request = self.wait_request().await;
        let response = self.process_request(request.command).await;
        request.respond(response);
    }

    /// Wait for sensor to receive a request
    pub async fn wait_request(&self) -> ipc::Request<'_, NoopRawMutex, Request, Response> {
        self.device.ipc.receive().await
    }

    /// Process sensor request
    pub async fn process_request(&self, request: Request) -> Response {
        match request {
            Request::GetTemp => {
                let temp = self.state.lock().await.samples.recent();
                Ok(ResponseData::Temp(temp))
            }
            Request::GetAvgTemp => {
                let temp = self.state.lock().await.samples.average();
                Ok(ResponseData::Temp(temp))
            }
            Request::SetAlertLow(low) => {
                self.driver
                    .lock()
                    .await
                    .set_temperature_threshold_low(low)
                    .await
                    .map_err(|_| Error::Hardware)?;

                self.state.lock().await.alert_low = low;
                Ok(ResponseData::Success)
            }
            Request::SetAlertHigh(high) => {
                self.driver
                    .lock()
                    .await
                    .set_temperature_threshold_high(high)
                    .await
                    .map_err(|_| Error::Hardware)?;

                self.state.lock().await.alert_high = high;
                Ok(ResponseData::Success)
            }
            Request::SetSamplingPeriod(period) => {
                self.state.lock().await.period = period;
                Ok(ResponseData::Success)
            }
            Request::Enable => {
                self.enable().await;
                Ok(ResponseData::Success)
            }
            Request::Disable => {
                self.disable().await;
                Ok(ResponseData::Success)
            }
        }
    }

    pub async fn handle_rx(&self) {
        loop {
            self.wait_and_process().await;
        }
    }

    /// Periodically samples temperature from physical sensor and caches it
    pub async fn handle_sampling(&self) {
        loop {
            // Only sample temperature if enabled
            if self
                .state
                .lock()
                .await
                .enabled
                .load(core::sync::atomic::Ordering::SeqCst)
            {
                match self.driver.lock().await.temperature().await {
                    Ok(temp) => self.state.lock().await.samples.push(temp),
                    Err(e) => error!("Error sampling temperature: {:?}", e.kind()),
                }

                let period = self.state.lock().await.period;
                Timer::after_millis(period).await;

            // Otherwise sleep and wait to be re-enabled
            } else {
                self.device.enable.wait().await;
            }
        }
    }

    /// Waits for a temperature threshold interrupt to be generated then notifies alert channel
    pub async fn handle_alert<A: embedded_hal_async::digital::Wait>(&self, mut alert_pin: A) {
        loop {
            if alert_pin.wait_for_falling_edge().await.is_err() {
                error!("Error awaiting alert pin interrupt");
            }

            match self.driver.lock().await.temperature().await {
                Ok(temp) => {
                    let alert = if temp <= self.state.lock().await.alert_low {
                        Alert::ThresholdLow
                    } else {
                        Alert::ThresholdHigh
                    };

                    self.device.alert.signal(alert);
                }
                Err(e) => error!("Error reading temperature after sensor alert: {:?}", e.kind()),
            }
        }
    }
}

/// This is a helper macro for implementing the sensor task since tasks cannot be generic
#[macro_export]
macro_rules! impl_sensor_task {
    ($sensor_task_name:ident, $sensor_type:ty, $alert_pin_type:ty) => {
        #[embassy_executor::task]
        pub async fn $sensor_task_name(
            sensor: &'static $crate::sensor::Sensor<$sensor_type>,
            mut alert_pin: $alert_pin_type,
        ) {
            embedded_services::info!("Sensor task started!");

            let _ = embassy_futures::select::select3(
                sensor.handle_rx(),
                sensor.handle_sampling(),
                sensor.handle_alert(alert_pin),
            )
            .await;
        }
    };
}
