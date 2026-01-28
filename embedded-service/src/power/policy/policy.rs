//! Context for any power policy implementations

use crate::GlobalRawMutex;
use crate::broadcaster::immediate as broadcaster;
use crate::power::policy::{CommsMessage, ConsumerPowerCapability, ProviderPowerCapability};
use embassy_sync::channel::Channel;

use super::charger::ChargerResponse;
use super::device::{self};
use super::{DeviceId, Error, action, charger};
use crate::power::policy::charger::ChargerResponseData::Ack;
use crate::{error, intrusive_list};

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
pub struct Context<const POLICY_CHANNEL_SIZE: usize> {
    /// Registered devices
    power_devices: intrusive_list::IntrusiveList,
    /// Policy request
    policy_request: Channel<GlobalRawMutex, Request, POLICY_CHANNEL_SIZE>,
    /// Policy response
    policy_response: Channel<GlobalRawMutex, InternalResponseData, POLICY_CHANNEL_SIZE>,
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
            power_devices: intrusive_list::IntrusiveList::new(),
            charger_devices: intrusive_list::IntrusiveList::new(),
            policy_request: Channel::new(),
            policy_response: Channel::new(),
            broadcaster: broadcaster::Immediate::new(),
        }
    }

    /// Register a device with the power policy service
    pub fn register_device(
        &self,
        device: &'static impl device::DeviceContainer<POLICY_CHANNEL_SIZE>,
    ) -> Result<(), intrusive_list::Error> {
        let device = device.get_power_policy_device();
        if self.get_device(device.id()).is_ok() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.power_devices.push(device)
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
        }

        Err(Error::InvalidDevice)
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
        self.policy_response.send(response).await
    }

    /// Provides access to the device list
    pub fn devices(&self) -> &intrusive_list::IntrusiveList {
        &self.power_devices
    }

    /// Provides access to the charger list
    pub fn chargers(&self) -> &intrusive_list::IntrusiveList {
        &self.charger_devices
    }

    /// Try to provide access to the actions available to the policy for the given state and device
    pub async fn try_policy_action<S: action::Kind>(
        &self,
        id: DeviceId,
    ) -> Result<action::policy::Policy<'_, S, POLICY_CHANNEL_SIZE>, Error> {
        self.get_device(id)?.try_policy_action().await
    }

    /// Provide access to current policy actions
    pub async fn policy_action(
        &self,
        id: DeviceId,
    ) -> Result<action::policy::AnyState<'_, POLICY_CHANNEL_SIZE>, Error> {
        Ok(self.get_device(id)?.policy_action().await)
    }

    /// Broadcast a power policy message to all subscribers
    pub async fn broadcast_message(&self, message: CommsMessage) {
        self.broadcaster.broadcast(message).await;
    }
}

/// Init power policy service
pub fn init() {}
