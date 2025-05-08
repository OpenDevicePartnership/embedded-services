//! Sensor Device
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embedded_sensors_hal_async::temperature::{TemperatureSensor, TemperatureThresholdWait};
use embedded_services::{intrusive_list, oem, Node};

/// Sensor error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// An invalid request was received
    InvalidRequest,
    /// Device encountered a hardware failure
    HardwareFailure,
}

/// Sensor request
// TODO: More generic requests that make sense
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// Current temperature measurement
    GetCurTemp,
    /// Set low alert thresholds
    SetThresholdLow(f32),
    /// Set high alert thresholds
    SetThresholdHigh(f32),

    /// OEM-specific request
    Oem(oem::Message),
}

/// Sensor response
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    /// Response for any request that is successful but does not require data
    Success,
    /// Current temperature of sensor in dC
    GetCurTemp(f32),

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
        Request::GetCurTemp => {
            let temp = controller.temperature().await.map_err(|_| Error::HardwareFailure)?;
            Ok(Response::GetCurTemp(temp))
        }
        Request::SetThresholdLow(_low) => todo!(),
        Request::SetThresholdHigh(_low) => todo!(),
        _ => Err(Error::InvalidRequest),
    }
}

/// Trait which driver implementers use to bridge gap between requests and hardware calls
#[allow(async_fn_in_trait)]
pub trait Controller: TemperatureSensor + TemperatureThresholdWait {
    async fn process_request(&mut self, request: Request) -> Result<Response, Error> {
        process_request(self, request).await
    }
}

/// Sensor device struct
pub struct Device {
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

/// Wrapper around Device for insertion into intrusive linked list
pub struct DeviceNode {
    /// Intrusive list node
    node: Node,
    /// Static reference to device
    pub device: &'static Device,
}

impl DeviceNode {
    pub fn new(device: &'static Device) -> Self {
        Self {
            node: Node::uninit(),
            device,
        }
    }
}

impl intrusive_list::NodeContainer for DeviceNode {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

/// Sensor struct containing device for comms and driver
pub struct Sensor<T: Controller> {
    /// Underlying device
    pub device: Device,
    /// Underlying controller
    pub controller: Mutex<NoopRawMutex, T>,
}

impl<T: Controller> Sensor<T> {
    /// New sensor
    pub fn new(id: DeviceId, controller: T) -> Self {
        Self {
            device: Device::new(id),
            controller: Mutex::new(controller),
        }
    }

    /// Process request for sensor
    pub async fn wait_and_process(&self) {
        let request = self.device.wait_request().await;
        let response = self.controller.lock().await.process_request(request).await;
        self.device.send_response(response).await;
    }
}

/// Should be called by a wrapper task per sensor (since tasks themselves cannot be generic)
pub async fn task<T: Controller>(sensor: &'static Sensor<T>) {
    loop {
        sensor.wait_and_process().await;
    }
}
