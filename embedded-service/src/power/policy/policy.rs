//! Context for any power policy implementations
use core::sync::atomic::{AtomicBool, Ordering};

use crate::GlobalRawMutex;
use crate::broadcaster::immediate as broadcaster;
use crate::power::policy::{CommsMessage, ConsumerPowerCapability, ProviderPowerCapability};
use embassy_sync::channel::Channel;

use super::charger::ChargerResponse;
use super::device::{self};
use super::{DeviceId, Error, action, charger};
use crate::power::policy::charger::ChargerResponseData::Ack;
use crate::{error, intrusive_list};

/// Number of slots for policy requests
const POLICY_CHANNEL_SIZE: usize = 1;

/// Data for a power policy request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Notify that a device has attached
    NotifyAttached,
    /// Notify that available power for consumption has changed
    NotifyConsumerCapability(Option<ConsumerPowerCapability>),
    /// Request the given amount of power to provider
    RequestProviderCapability(ProviderPowerCapability),
    /// Notify that a device cannot consume or provide power anymore
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

impl ResponseData {
    /// Returns an InvalidResponse error if the response is not complete
    pub fn complete_or_err(self) -> Result<(), Error> {
        match self {
            ResponseData::Complete => Ok(()),
        }
    }
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
    policy_request: Channel<GlobalRawMutex, Request, POLICY_CHANNEL_SIZE>,
    /// Policy response
    policy_response: Channel<GlobalRawMutex, InternalResponseData, POLICY_CHANNEL_SIZE>,
    /// Registered chargers
    chargers: intrusive_list::IntrusiveList,
    /// Message broadcaster
    broadcaster: broadcaster::Immediate<CommsMessage>,
}

impl Context {
    const fn new() -> Self {
        Self {
            devices: intrusive_list::IntrusiveList::new(),
            chargers: intrusive_list::IntrusiveList::new(),
            policy_request: Channel::new(),
            policy_response: Channel::new(),
            broadcaster: broadcaster::Immediate::new(),
        }
    }
}

static CONTEXT: Context = Context::new();

/// Init power policy service
pub fn init() {}

/// Register a device with the power policy service
pub fn register_device(device: &'static impl device::DeviceContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_power_policy_device();
    if get_device(device.id()).is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.devices.push(device)
}

/// Register a charger with the power policy service
pub fn register_charger(device: &'static impl charger::ChargerContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_charger();
    if get_charger(device.id()).is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.chargers.push(device)
}

/// Find a device by its ID
fn get_device(id: DeviceId) -> Option<&'static device::Device> {
    for device in &CONTEXT.devices {
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

/// Returns the total amount of power that is being supplied to external devices
pub async fn compute_total_provider_power_mw() -> u32 {
    let mut total = 0;
    for device in CONTEXT.devices.iter_only::<device::Device>() {
        if let Some(capability) = device.provider_capability().await {
            if device.is_provider().await {
                total += capability.capability.max_power_mw();
            }
        }
    }
    total
}

/// Find a device by its ID
fn get_charger(id: charger::ChargerId) -> Option<&'static charger::Device> {
    for charger in &CONTEXT.chargers {
        if let Some(data) = charger.data::<charger::Device>() {
            if data.id() == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in charger list");
        }
    }

    None
}

/// Convenience function to send a request to the power policy service
pub(super) async fn send_request(from: DeviceId, request: RequestData) -> Result<ResponseData, Error> {
    CONTEXT
        .policy_request
        .send(Request {
            id: from,
            data: request,
        })
        .await;
    CONTEXT.policy_response.receive().await
}

/// Initialize chargers in hardware
pub async fn init_chargers() -> ChargerResponse {
    for charger in &CONTEXT.chargers {
        if let Some(data) = charger.data::<charger::Device>() {
            data.execute_command(charger::PolicyEvent::InitRequest)
                .await
                .inspect_err(|e| error!("Charger {:?} failed InitRequest: {:?}", data.id(), e))?;
        }
    }
    Ok(Ack)
}

/// Check if charger hardware is ready for communications.
pub async fn check_chargers_ready() -> ChargerResponse {
    for charger in &CONTEXT.chargers {
        if let Some(data) = charger.data::<charger::Device>() {
            data.execute_command(charger::PolicyEvent::CheckReady)
                .await
                .inspect_err(|e| error!("Charger {:?} failed CheckReady: {:?}", data.id(), e))?;
        }
    }
    Ok(Ack)
}

/// Register a message receiver for power policy messages
pub fn register_message_receiver(
    receiver: &'static broadcaster::Receiver<'_, CommsMessage>,
) -> intrusive_list::Result<()> {
    CONTEXT.broadcaster.register_receiver(receiver)
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

    /// Initialize Policy charger devices
    pub async fn init() -> Result<(), Error> {
        // Check if the chargers are powered and able to communicate
        check_chargers_ready().await?;
        // Initialize chargers
        init_chargers().await?;

        Ok(())
    }

    /// Wait for a power policy request
    pub async fn wait_request(&self) -> Request {
        CONTEXT.policy_request.receive().await
    }

    /// Send a response to a power policy request
    pub async fn send_response(&self, response: Result<ResponseData, Error>) {
        CONTEXT.policy_response.send(response).await
    }

    /// Get a device by its ID
    pub fn get_device(&self, id: DeviceId) -> Result<&'static device::Device, Error> {
        get_device(id).ok_or(Error::InvalidDevice)
    }

    /// Provides access to the device list
    pub fn devices(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.devices
    }

    /// Get a charger by its ID
    pub fn get_charger(&self, id: charger::ChargerId) -> Result<&'static charger::Device, Error> {
        get_charger(id).ok_or(Error::InvalidDevice)
    }

    /// Provides access to the charger list
    pub fn chargers(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.chargers
    }

    /// Try to provide access to the actions available to the policy for the given state and device
    pub async fn try_policy_action<S: action::Kind>(
        &self,
        id: DeviceId,
    ) -> Result<action::policy::Policy<'_, S>, Error> {
        self.get_device(id)?.try_policy_action().await
    }

    /// Provide access to current policy actions
    pub async fn policy_action(&self, id: DeviceId) -> Result<action::policy::AnyState<'_>, Error> {
        Ok(self.get_device(id)?.policy_action().await)
    }

    /// Broadcast a power policy message to all subscribers
    pub async fn broadcast_message(&self, message: CommsMessage) {
        CONTEXT.broadcaster.broadcast(message).await;
    }
}
