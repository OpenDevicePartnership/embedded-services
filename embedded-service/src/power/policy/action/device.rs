//! Device state machine actions
use super::*;
use crate::info;
use crate::power::policy::{device, policy, Error, PowerCapability};

/// Device state machine control
pub struct Device<'a, S: Kind> {
    device: &'a device::Device,
    _state: core::marker::PhantomData<S>,
}

impl<'a, S: Kind> Device<'a, S> {
    /// Create a new state machine
    pub(crate) fn new(device: &'a device::Device) -> Self {
        Self {
            device,
            _state: core::marker::PhantomData,
        }
    }

    /// Detach the device
    pub async fn detach(self) -> Result<Device<'a, Detached>, Error> {
        info!("Received detach from device {}", self.device.id().0);
        self.device.set_state(device::State::Detached).await;
        self.device.update_consumer_capability(None).await;
        policy::send_request(self.device.id(), policy::RequestData::NotifyDetached)
            .await?
            .complete_or_err()?;
        Ok(Device::new(self.device))
    }

    /// Disconnect this device
    async fn disconnect_internal(&self) -> Result<(), Error> {
        info!("Device {} disconnecting", self.device.id().0);
        self.device.set_state(device::State::Idle).await;
        policy::send_request(self.device.id(), policy::RequestData::NotifyDisconnect)
            .await?
            .complete_or_err()
    }
}

impl<'a> Device<'a, Detached> {
    /// Attach the device
    pub async fn attach(self) -> Result<Device<'a, Idle>, Error> {
        info!("Received attach from device {}", self.device.id().0);
        self.device.set_state(device::State::Idle).await;
        policy::send_request(self.device.id(), policy::RequestData::NotifyAttached)
            .await?
            .complete_or_err()?;
        Ok(Device::new(self.device))
    }
}

impl<'a> Device<'a, Idle> {
    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
        info!(
            "Device {} consume capability updated {:#?}",
            self.device.id().0,
            capability
        );
        self.device.update_consumer_capability(capability).await;
        policy::send_request(
            self.device.id(),
            policy::RequestData::NotifyConsumerCapability(capability),
        )
        .await?
        .complete_or_err()
    }

    /// Request the given power from the power policy service
    pub async fn request_provider_power_capability(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Request provide from device {}, {:#?}", self.device.id().0, capability);
        policy::send_request(
            self.device.id(),
            policy::RequestData::RequestProviderCapability(capability),
        )
        .await?
        .complete_or_err()
    }
}

impl<'a> Device<'a, ConnectedConsumer> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }
}

impl<'a> Device<'a, ConnectedProvider> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }
}
