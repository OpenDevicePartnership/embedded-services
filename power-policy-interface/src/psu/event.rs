//! Messages originating from a PSU
use core::future::ready;

use embedded_services::{
    event::{NonBlockingSender, Sender},
    sync::Lockable,
};

use crate::{
    capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability},
    psu,
};

/// Data for an event broadcast from a PSU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EventData {
    /// Notify that a device has attached
    Attached,
    /// Notify that available power for consumption has changed
    UpdatedConsumerCapability(Option<ConsumerPowerCapability>),
    /// Request the given amount of power to provider
    RequestedProviderCapability(Option<ProviderPowerCapability>),
    /// Notify that a device cannot consume or provide power anymore
    Disconnected(ConsumerDisconnect),
    /// Notify that a device has detached
    Detached,
}

/// Event broadcast from a PSU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Event<'a, D: Lockable>
where
    D::Inner: psu::Psu,
{
    /// Device that sent this request
    pub psu: &'a D,
    /// Event data
    pub event: EventData,
}

/// New-type that implements the [`crate::psu::notification::Notifier`] trait for any [`NonBlockingSender<EventData>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct NonBlockingSenderNotifier<S: NonBlockingSender<EventData>>(pub S);

impl<S: NonBlockingSender<EventData>> crate::psu::notification::Notifier for NonBlockingSenderNotifier<S> {
    fn notify_attached(&mut self) -> impl Future<Output = Result<(), crate::psu::notification::Error>> {
        ready(
            self.0
                .try_send(EventData::Attached)
                .ok_or(crate::psu::notification::Error::WouldBlock),
        )
    }

    fn notify_updated_consumer_capability(
        &mut self,
        capability: Option<ConsumerPowerCapability>,
    ) -> impl Future<Output = Result<(), crate::psu::notification::Error>> {
        ready(
            self.0
                .try_send(EventData::UpdatedConsumerCapability(capability))
                .ok_or(crate::psu::notification::Error::WouldBlock),
        )
    }

    fn notify_requested_provider_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) -> impl Future<Output = Result<(), crate::psu::notification::Error>> {
        ready(
            self.0
                .try_send(EventData::RequestedProviderCapability(capability))
                .ok_or(crate::psu::notification::Error::WouldBlock),
        )
    }

    fn notify_disconnected(
        &mut self,
        flags: ConsumerDisconnect,
    ) -> impl Future<Output = Result<(), crate::psu::notification::Error>> {
        ready(
            self.0
                .try_send(EventData::Disconnected(flags))
                .ok_or(crate::psu::notification::Error::WouldBlock),
        )
    }

    fn notify_detached(&mut self) -> impl Future<Output = Result<(), crate::psu::notification::Error>> {
        ready(
            self.0
                .try_send(EventData::Detached)
                .ok_or(crate::psu::notification::Error::WouldBlock),
        )
    }
}

impl<S: NonBlockingSender<EventData>> From<S> for NonBlockingSenderNotifier<S> {
    fn from(sender: S) -> Self {
        Self(sender)
    }
}

/// New-type that implements the [`crate::psu::notification::Notifier`] trait for any [`Sender<EventData>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct SenderNotifier<S: Sender<EventData>>(pub S);

impl<S: Sender<EventData>> crate::psu::notification::Notifier for SenderNotifier<S> {
    async fn notify_attached(&mut self) -> Result<(), crate::psu::notification::Error> {
        self.0.send(EventData::Attached).await;
        Ok(())
    }

    async fn notify_updated_consumer_capability(
        &mut self,
        capability: Option<ConsumerPowerCapability>,
    ) -> Result<(), crate::psu::notification::Error> {
        self.0.send(EventData::UpdatedConsumerCapability(capability)).await;
        Ok(())
    }

    async fn notify_requested_provider_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) -> Result<(), crate::psu::notification::Error> {
        self.0.send(EventData::RequestedProviderCapability(capability)).await;
        Ok(())
    }

    async fn notify_disconnected(&mut self, flags: ConsumerDisconnect) -> Result<(), crate::psu::notification::Error> {
        self.0.send(EventData::Disconnected(flags)).await;
        Ok(())
    }

    async fn notify_detached(&mut self) -> Result<(), crate::psu::notification::Error> {
        self.0.send(EventData::Detached).await;
        Ok(())
    }
}

impl<S: Sender<EventData>> From<S> for SenderNotifier<S> {
    fn from(sender: S) -> Self {
        Self(sender)
    }
}
