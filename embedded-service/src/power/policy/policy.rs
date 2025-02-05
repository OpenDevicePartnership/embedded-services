//! Context for any power policy implementations
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;

use super::{device, DeviceId, Error, PowerCapability};
use crate::{error, intrusive_list};

/// Number of slots for policy requests
const POLICY_CHANNEL_SIZE: usize = 1;

/// Data for a power policy request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Notify that a device has attached
    NotifyAttached,
    /// Notify that available power for sinking has changed
    NotifySinkCapability(Option<PowerCapability>),
    /// Request the given amount of power to source
    RequestSourceCapability(PowerCapability),
    /// Notify that a device cannot source or sink power anymore
    NotifyDisconnect,
    /// Notify that a device has detached
    NotifyDetached,
}

/// Request to the power policy service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Request {
    /// Device that sent this request
    pub id: DeviceId,
    /// Request data
    pub data: RequestData,
}

/// Data for a power policy response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// The request was completed successfully
    Complete,
}

/// Response from the power policy service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response {
    /// Target device
    pub id: DeviceId,
    /// Response data
    pub data: ResponseData,
}

/// Wrapper type to make code cleaner
type InternalResponseData = Result<ResponseData, Error>;

/// Power policy context
struct Context {
    /// Registered devices
    devices: intrusive_list::IntrusiveList,
    /// Policy request
    policy_request: Channel<NoopRawMutex, Request, POLICY_CHANNEL_SIZE>,
    /// Policy response
    policy_response: Channel<NoopRawMutex, InternalResponseData, POLICY_CHANNEL_SIZE>,
}

impl Context {
    fn new() -> Self {
        Self {
            devices: intrusive_list::IntrusiveList::new(),
            policy_request: Channel::new(),
            policy_response: Channel::new(),
        }
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Init power policy service
pub fn init() {
    CONTEXT.get_or_init(Context::new);
}

/// Register a device with the power policy service
pub async fn register_device(device: &'static impl device::DeviceContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_power_policy_device();
    if get_device(device.id()).await.is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.get().await.devices.push(device)
}

/// Find a device by its ID
async fn get_device(id: DeviceId) -> Option<&'static device::Device> {
    for device in &CONTEXT.get().await.devices {
        if let Some(data) = device.data::<device::Device>() {
            if data.id() == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in devices list");
        }
    }

    None
}

/// Convenience function to send a request to the power policy service
pub(super) async fn send_request(from: DeviceId, request: RequestData) -> Result<ResponseData, Error> {
    let context = CONTEXT.get().await;
    context
        .policy_request
        .send(Request {
            id: from,
            data: request,
        })
        .await;
    context.policy_response.receive().await
}

/// Singleton struct to give access to the power policy context
pub struct ContextToken(());

impl ContextToken {
    /// Create a new context token, returning None if this function has been called before
    pub fn create() -> Option<Self> {
        static INIT: AtomicBool = AtomicBool::new(false);
        if INIT.load(Ordering::SeqCst) {
            return None;
        }

        INIT.store(true, Ordering::SeqCst);
        Some(ContextToken(()))
    }

    /// Wait for a power policy request
    pub async fn wait_request(&self) -> Request {
        CONTEXT.get().await.policy_request.receive().await
    }

    /// Send a response to a power policy request
    pub async fn send_response(&self, response: Result<ResponseData, Error>) {
        CONTEXT.get().await.policy_response.send(response).await
    }

    /// Get a device by its ID
    pub async fn get_device(&self, id: DeviceId) -> Result<&'static device::Device, Error> {
        get_device(id).await.ok_or(Error::InvalidDevice)
    }

    /// Provides access to the device list
    pub async fn devices(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.get().await.devices
    }
}
