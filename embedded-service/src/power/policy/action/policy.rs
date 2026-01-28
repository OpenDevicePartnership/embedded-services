//! Policy state machine
use embassy_time::{Duration, TimeoutError, with_timeout};

use super::*;
use crate::power::policy::{ConsumerPowerCapability, Error, ProviderPowerCapability, device};
use crate::{error, info};

/// Default timeout for device commands to prevent the policy from getting stuck
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// Policy state machine control
pub struct Policy<'a, S: Kind, const N: usize> {
    device: &'a device::Device<N>,
    _state: core::marker::PhantomData<S>,
}

/// Enum to contain any state
pub enum AnyState<'a, const N: usize> {
    /// Detached
    Detached(Policy<'a, Detached, N>),
    /// Idle
    Idle(Policy<'a, Idle, N>),
    /// Connected Consumer
    ConnectedConsumer(Policy<'a, ConnectedConsumer, N>),
    /// Connected Provider
    ConnectedProvider(Policy<'a, ConnectedProvider, N>),
}

impl<const N: usize> AnyState<'_, N> {
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

impl<'a, S: Kind, const N: usize> Policy<'a, S, N> {
    /// Create a new state machine
    pub(crate) fn new(device: &'a device::Device<N>) -> Self {
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
    async fn connect_as_provider_internal_no_timeout(&self, capability: ProviderPowerCapability) -> Result<(), Error> {
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
    async fn connect_provider_internal(&self, capability: ProviderPowerCapability) -> Result<(), Error> {
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
impl<const N: usize> Policy<'_, Detached, N> {}

impl<'a, const N: usize> Policy<'a, Idle, N> {
    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer, N>, Error> {
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
    pub async fn connect_consumer(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer, N>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider, N>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(
        self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider, N>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }
}

impl<'a, const N: usize> Policy<'a, ConnectedConsumer, N> {
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, Idle, N>, Error> {
        self.disconnect_internal_no_timeout()
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle, N>, Error> {
        self.disconnect_internal().await.map(|_| Policy::new(self.device))
    }
}

impl<'a, const N: usize> Policy<'a, ConnectedProvider, N> {
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, Idle, N>, Error> {
        if let Err(e) = self.disconnect_internal_no_timeout().await {
            error!("Error disconnecting device {}: {:?}", self.device.id().0, e);
            return Err(e);
        }
        Ok(Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, Idle, N>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.disconnect_no_timeout()).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer, N>, Error> {
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
    pub async fn connect_consumer(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, ConnectedConsumer, N>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        &self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider, N>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(
        &self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, ConnectedProvider, N>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Get the provider power capability of this device
    pub async fn power_capability(&self) -> Option<ProviderPowerCapability> {
        self.device.provider_capability().await
    }
}
