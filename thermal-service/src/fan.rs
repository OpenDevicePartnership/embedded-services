//! Fan Device
use crate::utils::SampleBuf;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_fans_async::{self as fan_traits, Error as HadrwareError};
use embedded_services::error;
use embedded_services::ipc::deferred as ipc;
use embedded_services::{intrusive_list, Node};

// RPM sample buffer size
const BUFFER_SIZE: usize = 16;

/// Convenience type for Fan response result
pub type Response = Result<ResponseData, Error>;

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
pub enum Request {
    /// Most recent RPM measurement
    GetRpm,
    /// Average RPM measurement
    GetAvgRpm,
    /// Get Min RPM
    GetMinRpm,
    /// Get Max RPM
    GetMaxRpm,
    /// Set RPM
    SetRpm(u16),
    /// Set RPM sampling period (in ms)
    SetSamplingPeriod(u64),
}

/// Fan response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// Response for any request that is successful but does not require data
    Success,
    /// RPM
    Rpm(u16),
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
    ipc: ipc::Channel<NoopRawMutex, Request, Response>,
}

impl Device {
    /// Create a new sensor device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: Node::uninit(),
            id,
            ipc: ipc::Channel::new(),
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

// Internal fan state
struct State {
    samples: SampleBuf<u16, BUFFER_SIZE>,
    period: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            samples: SampleBuf::create(),
            period: 200,
        }
    }
}

/// Fan struct containing device for comms and driver
pub struct Fan<T: fan_traits::Fan + fan_traits::RpmSense> {
    /// Underlying device
    device: Device,
    /// Underlying driver
    driver: Mutex<NoopRawMutex, T>,
    /// Underlying fan state
    state: Mutex<NoopRawMutex, State>,
}

impl<T: fan_traits::Fan + fan_traits::RpmSense> Fan<T> {
    /// New fan
    pub fn new(id: DeviceId, driver: T) -> Self {
        Self {
            device: Device::new(id),
            driver: Mutex::new(driver),
            state: Mutex::new(State::default()),
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
    pub async fn wait_request(&self) -> ipc::Request<'_, NoopRawMutex, Request, Response> {
        self.device.ipc.receive().await
    }

    /// Process fan request
    pub async fn process_request(&self, request: Request) -> Response {
        match request {
            Request::GetRpm => {
                let rpm = self.state.lock().await.samples.recent();
                Ok(ResponseData::Rpm(rpm))
            }
            Request::GetAvgRpm => {
                let rpm = self.state.lock().await.samples.average();
                Ok(ResponseData::Rpm(rpm))
            }
            Request::SetRpm(rpm) => {
                self.driver
                    .lock()
                    .await
                    .set_speed_rpm(rpm)
                    .await
                    .map_err(|_| Error::Hardware)?;
                Ok(ResponseData::Success)
            }
            Request::GetMinRpm => {
                let min_rpm = self.driver.lock().await.min_rpm();
                Ok(ResponseData::Rpm(min_rpm))
            }
            Request::GetMaxRpm => {
                let max_rpm = self.driver.lock().await.max_rpm();
                Ok(ResponseData::Rpm(max_rpm))
            }
            Request::SetSamplingPeriod(period) => {
                self.state.lock().await.period = period;
                Ok(ResponseData::Success)
            }
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
            match self.driver.lock().await.rpm().await {
                Ok(rpm) => self.state.lock().await.samples.push(rpm),
                Err(e) => error!("Error sampling rpm: {:?}", e.kind()),
            }

            let period = self.state.lock().await.period;
            Timer::after_millis(period).await;
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

            let _ = embassy_futures::select::select(fan.handle_rx(), fan.handle_sampling()).await;
        }
    };
}
