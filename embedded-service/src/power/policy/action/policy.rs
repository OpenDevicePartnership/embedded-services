use super::*;
use crate::info;
use crate::power::policy::{device, Error, PowerCapability};

/// Policy state machine control
pub struct Policy<'a, S: Kind> {
    device: &'a device::Device,
    _state: core::marker::PhantomData<S>,
}

impl<'a, S: Kind> Policy<'a, S> {
    /// Create a new state machine
    pub(crate) fn new(device: &'a device::Device) -> Self {
        Self {
            device,
            _state: core::marker::PhantomData,
        }
    }

    async fn disconnect_internal(&self) -> Result<(), Error> {
        info!("Device {} got disconnect request", self.device.id().0);
        self.device
            .execute_device_request(device::RequestData::Disconnect)
            .await?
            .complete_or_err()
    }
}

// The policy can do nothing when no device is attached
impl Policy<'_, Detached> {}

impl<'a> Policy<'a, Idle> {
    /// Connect this device as a consumer
    pub async fn connect_consumer(self, capability: PowerCapability) -> Result<Policy<'a, ConnectedConsumer>, Error> {
        info!("Device {} connecting consumer", self.device.id().0);

        self.device
            .execute_device_request(device::RequestData::ConnectConsumer(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedConsumer(capability))
            .await;
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(self, capability: PowerCapability) -> Result<Policy<'a, ConnectedProvider>, Error> {
        info!("Device {} connecting provider", self.device.id().0);

        self.device
            .execute_device_request(device::RequestData::ConnectProvider(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedProvider(capability))
            .await;
        Ok(Policy::new(self.device))
    }
}

impl<'a> Policy<'a, ConnectedConsumer> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Policy::new(self.device))
    }
}

impl<'a> Policy<'a, ConnectedProvider> {
    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle>, Error> {
        self.disconnect_internal().await?;
        Ok(Policy::new(self.device))
    }
}
