//! Policy state machine
use super::*;
use crate::power::policy::{device, Error, PowerCapability};
use crate::{error, info};

/// Policy state machine control
pub struct Policy<'a, S: Kind> {
    device: &'a device::Device,
    _state: core::marker::PhantomData<S>,
}

/// Enum to contain any state
pub enum AnyState<'a> {
    /// Detached
    Detached(Policy<'a, Detached>),
    /// Idle
    Idle(Policy<'a, Idle>),
    /// Connected Consumer
    ConnectedConsumer(Policy<'a, ConnectedConsumer>),
    /// Connected Provider
    ConnectedProvider(Policy<'a, ConnectedProvider>),
}

impl<'a> AnyState<'a> {
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
            .complete_or_err()?;
        self.device.set_state(device::State::Idle).await;
        self.device.exit_recovery().await;
        Ok(())
    }

    /// Connect this device as a provider
    async fn connect_provider_internal(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Device {} connecting provider", self.device.id().0);

        self.device
            .execute_device_request(device::RequestData::ConnectProvider(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedProvider(capability))
            .await;

        Ok(())
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
        self.connect_provider_internal(capability).await?;
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
        if let Err(e) = self.disconnect_internal().await {
            error!("Error disconnecting device {}: {:?}", self.device.id().0, e);
            self.device.enter_recovery().await;
            return Err(e);
        }
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(&self, capability: PowerCapability) -> Result<(), Error> {
        self.connect_provider_internal(capability).await
    }

    /// Get the provider power capability of this device
    pub async fn power_capability(&self) -> PowerCapability {
        self.device.provider_capability().await.unwrap()
    }
}
