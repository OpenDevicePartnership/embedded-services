//! Cfu Service related data structures and messages
//pub mod action;
pub mod component;

use core::sync::atomic::{AtomicBool, Ordering};

use crate::GlobalRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embedded_cfu_protocol::protocol_definitions::{CfuProtocolError, ComponentId};

use crate::cfu::component::{CfuDevice, CfuDeviceContainer, DEVICE_CHANNEL_SIZE, InternalResponseData, RequestData};
use crate::{error, intrusive_list};

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CfuError {
    /// Image did not pass validation
    BadImage,
    /// Component either doesn't exist
    InvalidComponent,
    /// Component is busy
    ComponentBusy,
    /// Component encountered a protocol error during execution
    ProtocolError(CfuProtocolError),
}

/// Request to the power policy service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Request {
    /// Component that sent this request
    pub id: ComponentId,
    /// Request data
    pub data: RequestData,
}

/// Cfu context
struct ClientContext {
    /// Registered devices
    devices: intrusive_list::IntrusiveList,
    /// Request to components
    request: Channel<GlobalRawMutex, Request, { DEVICE_CHANNEL_SIZE }>,
    /// Response from components
    response: Channel<GlobalRawMutex, InternalResponseData, { DEVICE_CHANNEL_SIZE }>,
}

impl ClientContext {
    fn new() -> Self {
        Self {
            devices: intrusive_list::IntrusiveList::new(),
            request: Channel::new(),
            response: Channel::new(),
        }
    }
}

static CONTEXT: OnceLock<ClientContext> = OnceLock::new();

/// Init Cfu Client service
pub fn init() {
    CONTEXT.get_or_init(ClientContext::new);
}

/// Register a device with the Cfu Client service
pub async fn register_device(device: &'static impl CfuDeviceContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_cfu_component_device();
    if get_device(device.component_id()).await.is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.get().await.devices.push(device)
}

/// Find a device by its ID
async fn get_device(id: ComponentId) -> Option<&'static CfuDevice> {
    for device in &CONTEXT.get().await.devices {
        if let Some(data) = device.data::<CfuDevice>() {
            if data.component_id() == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in devices list");
        }
    }

    None
}

/// Convenience function to send a request to the Cfu service
pub async fn send_request(from: ComponentId, request: RequestData) -> Result<InternalResponseData, CfuError> {
    let context = CONTEXT.get().await;
    context
        .request
        .send(Request {
            id: from,
            data: request,
        })
        .await;
    Ok(context.response.receive().await)
}

/// Convenience function to route a request to a specific component
pub async fn route_request(to: ComponentId, request: RequestData) -> Result<InternalResponseData, CfuError> {
    let device = get_device(to).await;
    if device.is_none() {
        return Err(CfuError::InvalidComponent);
    }
    device
        .unwrap()
        .execute_device_request(request)
        .await
        .map_err(CfuError::ProtocolError)
}

/// Send a request to the specific CFU device, but don't wait for a response
pub async fn send_device_request(to: ComponentId, request: RequestData) -> Result<(), CfuError> {
    let device = get_device(to).await;
    if device.is_none() {
        return Err(CfuError::InvalidComponent);
    }
    device.unwrap().send_request(request).await;
    Ok(())
}

/// Wait for a response from the specific CFU device
pub async fn wait_device_response(to: ComponentId) -> Result<InternalResponseData, CfuError> {
    let device = get_device(to).await;
    if device.is_none() {
        return Err(CfuError::InvalidComponent);
    }
    Ok(device.unwrap().wait_response().await)
}

/// Singleton struct to give access to the cfu client context
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

    /// Wait for a cfu request
    pub async fn wait_request(&self) -> Request {
        CONTEXT.get().await.request.receive().await
    }

    /// Send a response to a cfu request
    pub async fn send_response(&self, response: InternalResponseData) {
        CONTEXT.get().await.response.send(response).await
    }

    /// Get a device by its ID
    pub async fn get_device(&self, id: ComponentId) -> Result<&'static CfuDevice, CfuError> {
        get_device(id).await.ok_or(CfuError::InvalidComponent)
    }

    /// Provides access to the device list
    pub async fn devices(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.get().await.devices
    }
}
