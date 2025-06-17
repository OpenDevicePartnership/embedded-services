//! Device state machine actions
use super::*;
use crate::power::policy::{device, policy, Error, PowerCapability};
use crate::{info, trace};

/// Device state machine control
pub struct Device<'a, S: Kind> {
    device: &'a device::Device,
    _state: core::marker::PhantomData<S>,
}

/// Enum to contain any state
pub enum AnyState<'a> {
    /// Detached
    Detached(Device<'a, Detached>),
    /// Idle
    Idle(Device<'a, Idle>),
    /// Connected Consumer
    ConnectedConsumer(Device<'a, ConnectedConsumer>),
    /// Connected Provider
    ConnectedProvider(Device<'a, ConnectedProvider>),
}

impl AnyState<'_> {
    /// Return the kind of the contained state
    pub fn kind(&self) -> StateKind {
        match self {
            AnyState::Detached(_) => StateKind::Detached,
            AnyState::Idle(_) => StateKind::Idle,
            AnyState::ConnectedConsumer(_) => StateKind::ConnectedConsumer,
            AnyState::ConnectedProvider(_) => StateKind::ConnectedProvider,
        }
    }
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
        self.device.update_requested_provider_capability(None).await;
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

    /// Notify the power policy service of an updated consumer power capability
    async fn notify_consumer_power_capability_internal(
        &self,
        capability: Option<PowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "Device {} consume capability updated: {:#?}",
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
    async fn request_provider_power_capability_internal(&self, capability: PowerCapability) -> Result<(), Error> {
        if self.device.provider_capability().await == Some(capability) {
            // Already operating at this capability, power policy is already aware, don't need to do anything
            trace!("Device {} already requested: {:#?}", self.device.id().0, capability);
            return Ok(());
        }

        info!("Request provide from device {}, {:#?}", self.device.id().0, capability);
        self.device.update_requested_provider_capability(Some(capability)).await;
        policy::send_request(
            self.device.id(),
            policy::RequestData::RequestProviderCapability(capability),
        )
        .await?
        .complete_or_err()?;
        Ok(())
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

impl Device<'_, Idle> {
    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }

    /// Request the given power from the power policy service
    pub async fn request_provider_power_capability(&self, capability: PowerCapability) -> Result<(), Error> {
        self.request_provider_power_capability_internal(capability).await
    }
}

impl<'a> Device<'a, ConnectedConsumer> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }

    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }
}

impl<'a> Device<'a, ConnectedProvider> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }

    /// Request the given power from the power policy service
    pub async fn request_provider_power_capability(&self, capability: PowerCapability) -> Result<(), Error> {
        self.request_provider_power_capability_internal(capability).await
    }

    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }
}
