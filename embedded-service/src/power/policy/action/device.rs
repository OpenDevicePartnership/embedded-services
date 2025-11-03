//! Device state machine actions
use super::*;
use crate::power::policy::device::DeviceTrait;
use crate::power::policy::{ConsumerPowerCapability, Error, ProviderPowerCapability, device, policy};
use crate::sync::Lockable;
use crate::{info, trace};

/// Device state machine control
pub struct Device<'a, C: Lockable, S: Kind>
where
    C::Inner: DeviceTrait,
{
    device: &'a device::Device<'a, C>,
    _state: core::marker::PhantomData<S>,
}

/// Enum to contain any state
pub enum AnyState<'a, C: Lockable>
where
    C::Inner: DeviceTrait,
{
    /// Detached
    Detached(Device<'a, C, Detached>),
    /// Idle
    Idle(Device<'a, C, Idle>),
    /// Connected Consumer
    ConnectedConsumer(Device<'a, C, ConnectedConsumer>),
    /// Connected Provider
    ConnectedProvider(Device<'a, C, ConnectedProvider>),
}

impl<C: Lockable> AnyState<'_, C>
where
    C::Inner: DeviceTrait,
{
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

impl<'a, C: Lockable, S: Kind> Device<'a, C, S>
where
    C::Inner: DeviceTrait,
{
    /// Create a new state machine
    pub(crate) fn new(device: &'a device::Device<'a, C>) -> Self {
        Self {
            device,
            _state: core::marker::PhantomData,
        }
    }

    /// Detach the device
    pub async fn detach(self) -> Result<Device<'a, C, Detached>, Error> {
        info!("Received detach from device {}", self.device.id().0);
        self.device.set_state(device::State::Detached).await;
        self.device.update_consumer_capability(None).await;
        self.device.update_requested_provider_capability(None).await;
        self.device
            .context_ref
            .send_request(self.device.id(), policy::RequestData::NotifyDetached)
            .await?
            .complete_or_err()?;
        Ok(Device::new(self.device))
    }

    /// Disconnect this device
    async fn disconnect_internal(&self) -> Result<(), Error> {
        info!("Device {} disconnecting", self.device.id().0);
        self.device.update_consumer_capability(None).await;
        self.device.update_requested_provider_capability(None).await;
        self.device.set_state(device::State::Idle).await;
        self.device
            .context_ref
            .send_request(self.device.id(), policy::RequestData::NotifyDisconnect)
            .await?
            .complete_or_err()
    }

    /// Notify the power policy service of an updated consumer power capability
    async fn notify_consumer_power_capability_internal(
        &self,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        info!(
            "Device {} consume capability updated: {:#?}",
            self.device.id().0,
            capability
        );
        self.device.update_consumer_capability(capability).await;
        self.device
            .context_ref
            .send_request(
                self.device.id(),
                policy::RequestData::NotifyConsumerCapability(capability),
            )
            .await?
            .complete_or_err()
    }

    /// Request the given power from the power policy service
    async fn request_provider_power_capability_internal(
        &self,
        capability: ProviderPowerCapability,
    ) -> Result<(), Error> {
        if self.device.provider_capability().await == Some(capability) {
            // Already operating at this capability, power policy is already aware, don't need to do anything
            trace!("Device {} already requested: {:#?}", self.device.id().0, capability);
            return Ok(());
        }

        info!("Request provide from device {}, {:#?}", self.device.id().0, capability);
        self.device.update_requested_provider_capability(Some(capability)).await;
        self.device
            .context_ref
            .send_request(
                self.device.id(),
                policy::RequestData::RequestProviderCapability(capability),
            )
            .await?
            .complete_or_err()?;
        Ok(())
    }
}

impl<'a, C: Lockable> Device<'a, C, Detached>
where
    C::Inner: DeviceTrait,
{
    /// Attach the device
    pub async fn attach(self) -> Result<Device<'a, C, Idle>, Error> {
        info!("Received attach from device {}", self.device.id().0);
        self.device.set_state(device::State::Idle).await;
        self.device
            .context_ref
            .send_request(self.device.id(), policy::RequestData::NotifyAttached)
            .await?
            .complete_or_err()?;
        Ok(Device::new(self.device))
    }
}

impl<C: Lockable> Device<'_, C, Idle>
where
    C::Inner: DeviceTrait,
{
    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(
        &self,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }

    /// Request the given power from the power policy service
    pub async fn request_provider_power_capability(&self, capability: ProviderPowerCapability) -> Result<(), Error> {
        self.request_provider_power_capability_internal(capability).await
    }
}

impl<'a, C: Lockable> Device<'a, C, ConnectedConsumer>
where
    C::Inner: DeviceTrait,
{
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, C, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }

    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(
        &self,

        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }
}

impl<'a, C: Lockable> Device<'a, C, ConnectedProvider>
where
    C::Inner: DeviceTrait,
{
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Device<'a, C, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Device::new(self.device))
    }

    /// Request the given power from the power policy service
    pub async fn request_provider_power_capability(&self, capability: ProviderPowerCapability) -> Result<(), Error> {
        self.request_provider_power_capability_internal(capability).await
    }

    /// Notify the power policy service of an updated consumer power capability
    pub async fn notify_consumer_power_capability(
        &self,

        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), Error> {
        self.notify_consumer_power_capability_internal(capability).await
    }
}
