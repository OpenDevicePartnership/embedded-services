//! Fan Device
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embedded_fans_async as fan_traits;
use embedded_services::{intrusive_list, oem, Node};

/// Fan error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// An invalid request was received
    InvalidRequest,
    /// Device encountered a hardware failure
    HardwareFailure,
}

/// Fan request
// TODO: More generic requests that make sense
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// Get RPM
    GetRpm,
    /// Get Min RPM
    GetMinRpm,
    /// Get Max RPM
    GetMaxRpm,
    /// Set RPM
    SetRpm(u32),

    /// Get DBA
    GetDba,
    /// Get Min DBA
    GetMinDba,
    /// Get Max DBA
    GetMaxDba,

    /// Get Sones
    GetSones,
    /// Get Min Sones
    GetMinSones,
    /// Get Max Sones
    GetMaxSones,

    /// OEM-specific request
    Oem(oem::Message),
}

/// Fan response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    /// Response for any request that is successful but does not require data
    Success,
    /// Current RPM
    GetRpm(u32),
    /// Min RPM
    GetMinRpm(u32),
    /// Max RPM
    GetMaxRpm(u32),

    /// Get DBA
    GetDba(u32),
    /// Get Min DBA
    GetMinDba(u32),
    /// Get Max DBA
    GetMaxDba(u32),

    /// Get Sones
    GetSones(u32),
    /// Get Min Sones
    GetMinSones(u32),
    /// Get Max Sones
    GetMaxSones(u32),

    /// OEM-specific response
    Oem(oem::Message),
}

/// Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Generic process function which OEMs can still include in their trait override
pub async fn process_request<T: Controller + ?Sized>(controller: &mut T, request: Request) -> Result<Response, Error> {
    match request {
        Request::GetRpm => {
            let rpm = controller.rpm().await.map_err(|_| Error::HardwareFailure)?;
            Ok(Response::GetRpm(rpm as u32))
        }
        Request::SetRpm(rpm) => {
            controller
                .set_speed_rpm(rpm as u16)
                .await
                .map_err(|_| Error::HardwareFailure)?;
            Ok(Response::Success)
        }
        Request::GetMinRpm => {
            let min_rpm = controller.min_rpm();
            Ok(Response::GetMinRpm(min_rpm as u32))
        }
        Request::GetMaxRpm => {
            let max_rpm = controller.max_rpm();
            Ok(Response::GetMaxRpm(max_rpm as u32))
        }
        _ => Err(Error::InvalidRequest),
    }
}

/// Trait which driver implementers use to bridge gap between requests and hardware calls
#[allow(async_fn_in_trait)]
pub trait Controller: fan_traits::Fan + fan_traits::RpmSense {
    async fn process_request(&mut self, request: Request) -> Result<Response, Error> {
        process_request(self, request).await
    }
}

/// Fan device struct
pub struct Device {
    /// Intrusive list node
    node: Node,
    /// Device ID
    id: DeviceId,
    /// Channel for requests to the device
    request: Channel<NoopRawMutex, Request, 1>,
    /// Channel for responses from the device
    response: Channel<NoopRawMutex, Result<Response, Error>, 1>,
}

impl Device {
    /// Create a new sensor device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: Node::uninit(),
            id,
            request: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> Request {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: Result<Response, Error>) {
        self.response.send(response).await;
    }

    /// Execute request and wait for response
    pub async fn execute_request(&self, request: Request) -> Result<Response, Error> {
        self.request.send(request).await;
        self.response.receive().await
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

/// Fan struct containing device for comms and driver
pub struct Fan<T: Controller> {
    /// Underlying device
    pub device: Device,
    /// Underlying controller
    pub controller: Mutex<NoopRawMutex, T>,
}

impl<T: Controller> Fan<T> {
    /// New fan
    pub fn new(id: DeviceId, driver: T) -> Self {
        Self {
            device: Device::new(id),
            controller: Mutex::new(driver),
        }
    }

    /// Process request for fan
    pub async fn wait_and_process(&self) {
        let request = self.device.wait_request().await;
        let response = self.controller.lock().await.process_request(request).await;
        self.device.send_response(response).await;
    }
}

/// Should be called by a wrapper task per fan (since tasks themselves cannot be generic)
pub async fn task<T: Controller>(fan: &'static Fan<T>) {
    loop {
        fan.wait_and_process().await;
    }
}
