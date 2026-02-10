//! Context for any power policy implementations
use core::marker::PhantomData;
use core::pin::pin;

use crate::charger;
use crate::device::DeviceTrait;
use crate::device::event::Request;
use embassy_futures::select::select_slice;
use embedded_services::broadcaster::immediate as broadcaster;
use embedded_services::event::Receiver;
use embedded_services::sync::Lockable;

use crate::charger::ChargerResponse;
use crate::device::{self, DeviceId, Error, event::RequestData};
use crate::service::event::CommsMessage;
use embedded_services::{error, intrusive_list};

/// Power policy context
pub struct Context<D: Lockable, R: Receiver<RequestData>>
where
    D::Inner: DeviceTrait,
{
    /// Registered devices
    power_devices: intrusive_list::IntrusiveList,
    /// Registered chargers
    charger_devices: intrusive_list::IntrusiveList,
    /// Message broadcaster
    broadcaster: broadcaster::Immediate<CommsMessage>,
    _phantom: PhantomData<(D, R)>,
}

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> Default for Context<D, R>
where
    D::Inner: DeviceTrait,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> Context<D, R>
where
    D::Inner: DeviceTrait,
{
    /// Construct a new power policy Context
    pub const fn new() -> Self {
        Self {
            power_devices: intrusive_list::IntrusiveList::new(),
            charger_devices: intrusive_list::IntrusiveList::new(),
            broadcaster: broadcaster::Immediate::new(),
            _phantom: PhantomData,
        }
    }

    /// Register a power device with the service
    pub fn register_device(
        &self,
        device: &'static impl device::DeviceContainer<D, R>,
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
    pub fn get_device(&self, id: DeviceId) -> Result<&'static device::Device<'static, D, R>, Error> {
        for device in &self.power_devices {
            if let Some(data) = device.data::<device::Device<'static, D, R>>() {
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
        for device in self.power_devices.iter_only::<device::Device<'static, D, R>>() {
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

    /// Initialize chargers in hardware
    pub async fn init_chargers(&self) -> ChargerResponse {
        for charger in &self.charger_devices {
            if let Some(data) = charger.data::<charger::Device>() {
                data.execute_command(charger::PolicyEvent::InitRequest)
                    .await
                    .inspect_err(|e| error!("Charger {:?} failed InitRequest: {:?}", data.id(), e))?;
            }
        }

        Ok(charger::ChargerResponseData::Ack)
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
        Ok(charger::ChargerResponseData::Ack)
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
        for device in self.devices().iter_only::<device::Device<'static, D, R>>() {
            // TODO: Validate Vec size at compile time
            if futures
                .push(async { device.receiver.lock().await.wait_next().await })
                .is_err()
            {
                error!("Futures vec overflow");
            }
        }

        let (event, index) = select_slice(pin!(&mut futures)).await;
        // Panic safety: The index is guaranteed to be within bounds since it comes from the select_slice result
        #[allow(clippy::unwrap_used)]
        let device = self
            .devices()
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
