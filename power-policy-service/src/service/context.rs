//! Context for any power policy implementations
use crate::charger;
use embedded_services::broadcaster::immediate as broadcaster;

use crate::charger::ChargerResponse;
use crate::psu::Error;
use crate::service::event::CommsMessage;
use embedded_services::{error, intrusive_list};

/// Power policy context
pub struct Context {
    /// Registered chargers
    charger_devices: intrusive_list::IntrusiveList,
    /// Message broadcaster
    broadcaster: broadcaster::Immediate<CommsMessage>,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Construct a new power policy Context
    pub const fn new() -> Self {
        Self {
            charger_devices: intrusive_list::IntrusiveList::new(),
            broadcaster: broadcaster::Immediate::new(),
        }
    }

    /// Register a charger with the power policy service
    pub fn register_charger(
        &self,
        charger: &'static impl charger::ChargerContainer,
    ) -> Result<(), intrusive_list::Error> {
        let charger = charger.get_charger();
        if self.get_charger(charger.id()).is_ok() {
            return Err(intrusive_list::Error::NodeAlreadyInList);
        }

        self.charger_devices.push(charger)
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

    /// Provides access to the charger list
    pub fn charger_devices(&self) -> &intrusive_list::IntrusiveList {
        &self.charger_devices
    }

    /// Broadcast a power policy message to all subscribers
    pub async fn broadcast_message(&self, message: CommsMessage) {
        self.broadcaster.broadcast(message).await;
    }
}
