//! Sensor Device
use crate::utils::SampleBuf;
use core::sync::atomic::AtomicBool;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_sensors_hal_async::sensor::Error as HardwareError;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdWait};
use embedded_services::ipc::deferred as ipc;
use embedded_services::{error, info};
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
pub struct Sensor<T: TemperatureSensor + TemperatureThresholdWait> {
    /// Underlying device
    device: Device,
    /// Underlying driver
    driver: Mutex<NoopRawMutex, T>,
    /// Underlying sensor state
    state: Mutex<NoopRawMutex, State>,
}

impl<T: TemperatureSensor + TemperatureThresholdWait> Sensor<T> {
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

    // Waits for a sensor request then processes it
    async fn wait_and_process(&self) {
        let request = self.device.ipc.receive().await;
        let response = self.process_request(request.command).await;
        request.respond(response);
    }

    // Processes request by making actual driver calls
    async fn process_request(&self, request: Request) -> Result<ResponseData, Error> {
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
}

// These should be called by a wrapper task per sensor (since tasks themselves cannot be generic, cannot define task here)
// TODO: Add macro to make that easier, and a macro that spawns all tasks

/// Waits for a IPC request, then processes it
pub async fn rx_task<T: TemperatureSensor + TemperatureThresholdWait>(sensor: &'static Sensor<T>) {
    info!("Sensor RX task started");

    loop {
        sensor.wait_and_process().await;
    }
}

/// Periodically samples temperature from physical sensor and caches it
pub async fn sample_task<T: TemperatureSensor + TemperatureThresholdWait>(sensor: &'static Sensor<T>) {
    info!("Sensor sample task started");

    loop {
        // Only sample temperature if enabled
        if sensor
            .state
            .lock()
            .await
            .enabled
            .load(core::sync::atomic::Ordering::SeqCst)
        {
            match sensor.driver.lock().await.temperature().await {
                Ok(temp) => sensor.state.lock().await.samples.push(temp),
                Err(e) => error!("Error sampling temperature: {:?}", e.kind()),
            }

            let period = sensor.state.lock().await.period;
            Timer::after_millis(period).await;

        // Otherwise sleep and wait to be re-enabled
        } else {
            sensor.device.enable.wait().await;
        }
    }
}

/// Waits for a temperature threshold interrupt to be generated then notifies alert channel
pub async fn alert_task<T: TemperatureSensor + TemperatureThresholdWait, A: embedded_hal_async::digital::Wait>(
    sensor: &'static Sensor<T>,
    mut alert_pin: A,
) {
    info!("Sensor alert task started");

    loop {
        if alert_pin.wait_for_falling_edge().await.is_err() {
            error!("Error awaiting alert pin interrupt");
        }

        match sensor.driver.lock().await.temperature().await {
            Ok(temp) => {
                let alert = if temp <= sensor.state.lock().await.alert_low {
                    Alert::ThresholdLow
                } else {
                    Alert::ThresholdHigh
                };

                sensor.device.alert.signal(alert);
            }
            Err(e) => error!("Error reading temperature after sensor alert: {:?}", e.kind()),
        }
    }
}
