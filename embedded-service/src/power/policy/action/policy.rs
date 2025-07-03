//! Policy state machine
use embassy_time::{Duration, TimeoutError, with_timeout};

use super::*;
use crate::power::policy::{Error, PowerCapability, device};
use crate::{error, info};

/// Default timeout for device commands to prevent the policy from getting stuck
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

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

impl<'a, S: Kind> Policy<'a, S> {
    /// Create a new state machine
    pub(crate) fn new(device: &'a device::Device) -> Self {
        Self {
            device,
            _state: core::marker::PhantomData,
        }
    }

    /// Common disconnect function used by multiple states
    async fn disconnect_internal_no_timeout(&self) -> Result<(), Error> {
        info!("Device {} got disconnect request", self.device.id().0);
        self.device
            .execute_device_command(device::CommandData::Disconnect)
            .await?
            .complete_or_err()?;
        self.device.set_state(device::State::Idle).await;
        Ok(())
    }

    /// Common disconnect function used by multiple states
    async fn disconnect_internal(&self) -> Result<(), Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.disconnect_internal_no_timeout()).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Common connect as provider function used by multiple states
    async fn connect_as_provider_internal_no_timeout(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Device {} connecting provider", self.device.id().0);

        self.device
            .execute_device_command(device::CommandData::ConnectAsProvider(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedProvider(capability))
            .await;

        Ok(())
    }

    /// Common connect provider function used by multiple states
    async fn connect_provider_internal(&self, capability: PowerCapability) -> Result<(), Error> {
        match with_timeout(
            DEFAULT_TIMEOUT,
            self.connect_as_provider_internal_no_timeout(capability),
        )
        .await
        {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }
}

// The policy can do nothing when no device is attached
impl Policy<'_, Detached> {}

impl<'a> Policy<'a, Idle> {
    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: PowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer>, Error> {
        info!("Device {} connecting as consumer", self.device.id().0);

        self.device
            .execute_device_command(device::CommandData::ConnectAsConsumer(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedConsumer(capability))
            .await;
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a consumer
    pub async fn connect_consumer(self, capability: PowerCapability) -> Result<Policy<'a, ConnectedConsumer>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        self,
        capability: PowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(self, capability: PowerCapability) -> Result<Policy<'a, ConnectedProvider>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }
}

impl<'a> Policy<'a, ConnectedConsumer> {
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, Idle>, Error> {
        self.disconnect_internal_no_timeout()
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle>, Error> {
        self.disconnect_internal().await.map(|_| Policy::new(self.device))
    }
}

impl<'a> Policy<'a, ConnectedProvider> {
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, Idle>, Error> {
        if let Err(e) = self.disconnect_internal_no_timeout().await {
            error!("Error disconnecting device {}: {:?}", self.device.id().0, e);
            return Err(e);
        }
        Ok(Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.disconnect_no_timeout()).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: PowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer>, Error> {
        info!("Device {} connecting as consumer", self.device.id().0);

        self.device
            .execute_device_command(device::CommandData::ConnectAsConsumer(capability))
            .await?
            .complete_or_err()?;

        self.device
            .set_state(device::State::ConnectedConsumer(capability))
            .await;
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a consumer
    pub async fn connect_consumer(self, capability: PowerCapability) -> Result<Policy<'a, ConnectedConsumer>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        &self,
        capability: PowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(&self, capability: PowerCapability) -> Result<Policy<'a, ConnectedProvider>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Get the provider power capability of this device
    pub async fn power_capability(&self) -> PowerCapability {
        self.device.provider_capability().await.unwrap()
    }
}
