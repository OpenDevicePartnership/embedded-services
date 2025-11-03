//! Policy state machine
use embassy_time::{Duration, TimeoutError, with_timeout};

use super::*;
use crate::power::policy::device::DeviceTrait;
use crate::power::policy::{ConsumerPowerCapability, Error, ProviderPowerCapability, device};
use crate::sync::Lockable;
use crate::{error, info};

/// Default timeout for device commands to prevent the policy from getting stuck
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// Policy state machine control
pub struct Policy<'a, C: Lockable, S: Kind>
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
    Detached(Policy<'a, C, Detached>),
    /// Idle
    Idle(Policy<'a, C, Idle>),
    /// Connected Consumer
    ConnectedConsumer(Policy<'a, C, ConnectedConsumer>),
    /// Connected Provider
    ConnectedProvider(Policy<'a, C, ConnectedProvider>),
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

impl<'a, C: Lockable, S: Kind> Policy<'a, C, S>
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

    /// Common disconnect function used by multiple states
    async fn disconnect_internal_no_timeout(&self) -> Result<(), Error> {
        info!("Device {} got disconnect request", self.device.id().0);
        self.device.device.lock().await.disconnect().await?;
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
        self.device.device.lock().await.connect_provider(capability).await?;
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
impl<C: Lockable> Policy<'_, C, Detached> where C::Inner: DeviceTrait {}

impl<'a, C: Lockable> Policy<'a, C, Idle>
where
    C::Inner: DeviceTrait,
{
    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedConsumer>, Error> {
        info!("Device {} connecting as consumer", self.device.id().0);

        self.device.device.lock().await.connect_consumer(capability).await?;

        self.device
            .set_state(device::State::ConnectedConsumer(capability))
            .await;
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a consumer
    pub async fn connect_consumer(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedConsumer>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedProvider>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(
        self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedProvider>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }
}

impl<'a, C: Lockable> Policy<'a, C, ConnectedConsumer>
where
    C::Inner: DeviceTrait,
{
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, C, Idle>, Error> {
        self.disconnect_internal_no_timeout()
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, C, Idle>, Error> {
        self.disconnect_internal().await.map(|_| Policy::new(self.device))
    }
}

impl<'a, C: Lockable> Policy<'a, C, ConnectedProvider>
where
    C::Inner: DeviceTrait,
{
    /// Disconnect this device
    pub async fn disconnect_no_timeout(self) -> Result<Policy<'a, C, Idle>, Error> {
        if let Err(e) = self.disconnect_internal_no_timeout().await {
            error!("Error disconnecting device {}: {:?}", self.device.id().0, e);
            return Err(e);
        }
        Ok(Policy::new(self.device))
    }

    /// Disconnect this device
    pub async fn disconnect(self) -> Result<Policy<'a, C, Idle>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.disconnect_no_timeout()).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a consumer
    pub async fn connect_as_consumer_no_timeout(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedConsumer>, Error> {
        info!("Device {} connecting as consumer", self.device.id().0);
        self.device.device.lock().await.connect_consumer(capability).await?;
        self.device
            .set_state(device::State::ConnectedConsumer(capability))
            .await;
        Ok(Policy::new(self.device))
    }

    /// Connect this device as a consumer
    pub async fn connect_consumer(
        self,
        capability: ConsumerPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedConsumer>, Error> {
        match with_timeout(DEFAULT_TIMEOUT, self.connect_as_consumer_no_timeout(capability)).await {
            Ok(r) => r,
            Err(TimeoutError) => Err(Error::Timeout),
        }
    }

    /// Connect this device as a provider
    pub async fn connect_provider_no_timeout(
        &self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedProvider>, Error> {
        self.connect_as_provider_internal_no_timeout(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Connect this device as a provider
    pub async fn connect_provider(
        &self,
        capability: ProviderPowerCapability,
    ) -> Result<Policy<'a, C, ConnectedProvider>, Error> {
        self.connect_provider_internal(capability)
            .await
            .map(|_| Policy::new(self.device))
    }

    /// Get the provider power capability of this device
    pub async fn power_capability(&self) -> Option<ProviderPowerCapability> {
        self.device.provider_capability().await
    }
}
