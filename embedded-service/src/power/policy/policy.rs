//! Context for any power policy implementations
<<<<<<< HEAD
=======
use core::marker::PhantomData;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, Ordering};
>>>>>>> 34701dd (Add device and receiver arguments to context token)

use crate::broadcaster::immediate as broadcaster;
use crate::event::{self, Receiver};
use crate::power::policy::device::DeviceTrait;
use crate::power::policy::{CommsMessage, ConsumerPowerCapability, ProviderPowerCapability};
use crate::sync::Lockable;
use embassy_futures::select::select_slice;
use embassy_sync::once_lock::OnceLock;

use super::charger::ChargerResponse;
use super::device::{self};
use super::{DeviceId, Error, charger};
use crate::power::policy::charger::ChargerResponseData::Ack;
use crate::{error, intrusive_list};

/// Data for a power policy request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Notify that a device has attached
    Attached,
    /// Notify that available power for consumption has changed
    UpdatedConsumerCapability(Option<ConsumerPowerCapability>),
    /// Request the given amount of power to provider
    RequestedProviderCapability(Option<ProviderPowerCapability>),
    /// Notify that a device cannot consume or provide power anymore
    Disconnected,
    /// Notify that a device has detached
    Detached,
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

/// Trait used by devices to send events to a power policy implementation
pub trait Sender: event::Sender<RequestData> {
    /// Wrapper to simplify sending this event
    fn on_attach(&mut self) -> impl Future<Output = ()> {
        self.send(RequestData::Attached)
    }

    /// Wrapper to simplify attempting to send this event
    fn try_on_update_consumer_capability(&mut self, cap: Option<ConsumerPowerCapability>) -> Option<()> {
        self.try_send(RequestData::UpdatedConsumerCapability(cap))
    }

    /// Wrapper to simplify sending this event
    fn on_update_consumer_capability(&mut self, cap: Option<ConsumerPowerCapability>) -> impl Future<Output = ()> {
        self.send(RequestData::UpdatedConsumerCapability(cap))
    }

    /// Wrapper to simplify attempting to send this event
    fn try_on_request_provider_capability(&mut self, cap: Option<ProviderPowerCapability>) -> Option<()> {
        self.try_send(RequestData::RequestedProviderCapability(cap))
    }

    /// Wrapper to simplify sending this event
    fn on_request_provider_capability(&mut self, cap: Option<ProviderPowerCapability>) -> impl Future<Output = ()> {
        self.send(RequestData::RequestedProviderCapability(cap))
    }

    /// Wrapper to simplify attempting to send this event
    fn try_on_disconnect(&mut self) -> Option<()> {
        self.try_send(RequestData::Disconnected)
    }

    /// Wrapper to simplify sending this event
    fn on_disconnect(&mut self) -> impl Future<Output = ()> {
        self.send(RequestData::Disconnected)
    }

    /// Wrapper to simplify attempting to send this event
    fn try_on_detach(&mut self) -> Option<()> {
        self.try_send(RequestData::Detached)
    }

    /// Wrapper to simplify sending this event
    fn on_detach(&mut self) -> impl Future<Output = ()> {
        self.send(RequestData::Detached)
    }
}

impl<T> Sender for T where T: event::Sender<RequestData> {}

/// Power policy context
pub struct Context {
    /// Registered devices
    power_devices: intrusive_list::IntrusiveList,
    /// Registered chargers
    charger_devices: intrusive_list::IntrusiveList,
    /// Message broadcaster
    broadcaster: broadcaster::Immediate<CommsMessage>,
}

impl<const POLICY_CHANNEL_SIZE: usize> Default for Context<POLICY_CHANNEL_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const POLICY_CHANNEL_SIZE: usize> Context<POLICY_CHANNEL_SIZE> {
    /// Construct a new power policy Context
    pub const fn new() -> Self {
        Self {
            devices: intrusive_list::IntrusiveList::new(),
            chargers: intrusive_list::IntrusiveList::new(),
            broadcaster: broadcaster::Immediate::default(),
        }
    }


/// Init power policy service
pub fn init() {}

/// Register a device with the power policy service
pub async fn register_device<C: Lockable + 'static, R: Receiver<RequestData> + 'static>(
    device: &'static impl device::DeviceContainer<C, R>,
) -> Result<(), intrusive_list::Error>
where
    C::Inner: DeviceTrait,
{
    let device = device.get_power_policy_device();
    if get_device::<C, R>(device.id()).await.is_some() {
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
async fn get_device<C: Lockable + 'static, R: Receiver<RequestData> + 'static>(
    id: DeviceId,
) -> Option<&'static device::Device<'static, C, R>>
where
    C::Inner: DeviceTrait,
{
    for device in &CONTEXT.get().await.devices {
        if let Some(data) = device.data::<device::Device<'static, C, R>>() {
            if data.id() == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in devices list");
        }

        self.power_devices.push(device)
    }
}

    /// Register a charger with the power policy service
    pub fn register_charger(
        &self,
        device: &'static impl charger::ChargerContainer,
    ) -> Result<(), intrusive_list::Error> {
        let device = device.get_charger();
        if self.get_charger(device.id()).is_ok() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.charger_devices.push(device)
    }

    /// Get a device by its ID
    pub fn get_device(&self, id: DeviceId) -> Result<&'static device::Device<POLICY_CHANNEL_SIZE>, Error> {
        for device in &self.power_devices {
            if let Some(data) = device.data::<device::Device<POLICY_CHANNEL_SIZE>>() {
                if data.id() == id {
                    return Ok(data);
                }
            } else {
                error!("Non-device located in devices list");
            }
        }

        Err(Error::InvalidDevice)
    }

    /// Returns the total amount of power that is being supplied to external devices
    pub async fn compute_total_provider_power_mw(&self) -> u32 {
        let mut total = 0;
        for device in self.power_devices.iter_only::<device::Device<POLICY_CHANNEL_SIZE>>() {
            if let Some(capability) = device.provider_capability().await {
                if device.is_provider().await {
                    total += capability.capability.max_power_mw();
                }
            }
        }
        total
    }

    /// Get a charger by its ID
    pub fn get_charger(&self, id: charger::ChargerId) -> Result<&'static charger::Device, Error> {
        for charger in &self.charger_devices {
            if let Some(data) = charger.data::<charger::Device>() {
                if data.id() == id {
                    return Ok(data);
                }
            } else {
                error!("Non-device located in charger list");
            }
    None
}

/// Initialize chargers in hardware
pub async fn init_chargers() -> ChargerResponse {
    for charger in &CONTEXT.chargers {
        if let Some(data) = charger.data::<charger::Device>() {
            data.execute_command(charger::PolicyEvent::InitRequest)
                .await
                .inspect_err(|e| error!("Charger {:?} failed InitRequest: {:?}", data.id(), e))?;
        }

        Err(Error::InvalidDevice)
    }
}

    /// Convenience function to send a request to the power policy service
    pub(super) async fn send_request(&self, from: DeviceId, request: RequestData) -> Result<ResponseData, Error> {
        self.policy_request
            .send(Request {
                id: from,
                data: request,
            })
            .await;
        self.policy_response.receive().await
    }

    /// Initialize chargers in hardware
    pub async fn init_chargers(&self) -> ChargerResponse {
        for charger in &self.charger_devices {
            if let Some(data) = charger.data::<charger::Device>() {
                data.execute_command(charger::PolicyEvent::InitRequest)
                    .await
                    .inspect_err(|e| error!("Charger {:?} failed InitRequest: {:?}", data.id(), e))?;
            }
        }

        Ok(Ack)
    }
}

/// Register a message receiver for power policy messages
pub fn register_message_receiver(
    receiver: &'static broadcaster::Receiver<'_, CommsMessage>,
) -> intrusive_list::Result<()> {
    CONTEXT.broadcaster.register_receiver(receiver)
}

/// Singleton struct to give access to the power policy context
pub struct ContextToken<D: Lockable, R: Receiver<RequestData>>
where
    D::Inner: DeviceTrait,
{
    _phantom: PhantomData<(D, R)>,
}

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> ContextToken<D, R>
where
    D::Inner: DeviceTrait,
{
    /// Create a new context token, returning None if this function has been called before
    pub fn create() -> Option<Self> {
        static INIT: AtomicBool = AtomicBool::new(false);
        if INIT.load(Ordering::SeqCst) {
            return None;
        }
        Ok(Ack)
    }

    /// Check if charger hardware is ready for communications.
    pub async fn check_chargers_ready(&self) -> ChargerResponse {
        for charger in &self.charger_devices {
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
        &self,
        receiver: &'static broadcaster::Receiver<'_, CommsMessage>,
    ) -> intrusive_list::Result<()> {
        self.broadcaster.register_receiver(receiver)
        Some(ContextToken { _phantom: PhantomData })
    }

    /// Initialize Policy charger devices
    pub async fn init(&self) -> Result<(), Error> {
        // Check if the chargers are powered and able to communicate
        self.check_chargers_ready().await?;
        // Initialize chargers
        self.init_chargers().await?;

        Ok(())
    }

    /// Wait for a power policy request
    pub async fn wait_request(&self) -> Request {
        self.policy_request.receive().await
    }

    /// Send a response to a power policy request
    pub async fn send_response(&self, response: Result<ResponseData, Error>) {
        CONTEXT.policy_response.send(response).await
    }

    /// Get a device by its ID
    pub async fn get_device(&self, id: DeviceId) -> Result<&'static device::Device<'static, D, R>, Error> {
        get_device(id).await.ok_or(Error::InvalidDevice)
    }

    /// Provides access to the device list
    pub fn devices(&self) -> &intrusive_list::IntrusiveList {
        &self.power_devices
    }

    /// Provides access to the charger list
    pub fn chargers(&self) -> &intrusive_list::IntrusiveList {
        &self.charger_devices
    }

    /// Broadcast a power policy message to all subscribers
    pub async fn broadcast_message(&self, message: CommsMessage) {
        self.broadcaster.broadcast(message).await;
    }

    /// Get the next pending device event
    pub async fn wait_request(&self) -> Request {
        let mut futures = heapless::Vec::<_, 16>::new();
        for device in self.devices().await.iter_only::<device::Device<'static, D, R>>() {
            // TODO: check this at compile time
            let _ = futures.push(async { device.receiver.lock().await.wait_next().await });
        }

        let (event, index) = select_slice(pin!(&mut futures)).await;
        let device = self
            .devices()
            .await
            .iter_only::<device::Device<'static, D, R>>()
            .nth(index)
            .unwrap();
        Request {
            id: device.id(),
            data: event,
        }
    }
}

/// Init power policy service
pub fn init() {}
