use core::{future::ready, marker::PhantomData};

use embedded_services::{
    event::{NonBlockingSender, Sender},
    sync::Lockable,
};

use crate::{
    capability::{ConsumerDisconnect, ConsumerPowerCapability, ProviderPowerCapability},
    psu::Psu,
    service::UnconstrainedState,
};

/// Event data broadcast from the service.
///
/// This enum doesn't contain a reference to the device and is suitable
/// for receivers that don't need to know which device triggered the event
/// and allows for receivers that don't need to be generic over the device type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EventData {
    /// Consumer disconnected
    ConsumerDisconnected(ConsumerDisconnect),
    /// Consumer connected
    ConsumerConnected(ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected,
    /// Provider connected
    ProviderConnected(ProviderPowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

impl<'device, PSU: Lockable> From<Event<'device, PSU>> for EventData
where
    PSU::Inner: Psu,
{
    fn from(value: Event<'device, PSU>) -> Self {
        match value {
            Event::ConsumerDisconnected(_, flags) => EventData::ConsumerDisconnected(flags),
            Event::ConsumerConnected(_, capability) => EventData::ConsumerConnected(capability),
            Event::ProviderDisconnected(_) => EventData::ProviderDisconnected,
            Event::ProviderConnected(_, capability) => EventData::ProviderConnected(capability),
            Event::Unconstrained(unconstrained) => EventData::Unconstrained(unconstrained),
        }
    }
}

/// Events broadcast from the service.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event<'device, PSU: Lockable>
where
    PSU::Inner: Psu,
{
    /// Consumer disconnected
    ConsumerDisconnected(&'device PSU, ConsumerDisconnect),
    /// Consumer connected
    ConsumerConnected(&'device PSU, ConsumerPowerCapability),
    /// Provider disconnected
    ProviderDisconnected(&'device PSU),
    /// Provider connected
    ProviderConnected(&'device PSU, ProviderPowerCapability),
    /// Unconstrained state changed
    Unconstrained(UnconstrainedState),
}

impl<'device, PSU> Clone for Event<'device, PSU>
where
    PSU: Lockable,
    PSU::Inner: Psu,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<'device, PSU> Copy for Event<'device, PSU>
where
    PSU: Lockable,
    PSU::Inner: Psu,
{
}

/// New-type that implements the [`crate::service::notification::Notifier`] trait for any [`NonBlockingSender<Event>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct NonBlockingSenderNotifier<
    'device,
    PSU: Lockable<Inner: Psu> + 'device,
    S: NonBlockingSender<Event<'device, PSU>>,
> {
    pub sender: S,
    _phantom: PhantomData<&'device PSU>,
}

impl<'device, PSU: Lockable<Inner: Psu>, S: NonBlockingSender<Event<'device, PSU>>>
    NonBlockingSenderNotifier<'device, PSU, S>
{
    /// Create a new [`NonBlockingSenderNotifier`]
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}

impl<'device, PSU: Lockable<Inner: Psu>, S: NonBlockingSender<Event<'device, PSU>>>
    crate::service::notification::Notifier<'device> for NonBlockingSenderNotifier<'device, PSU, S>
{
    type Psu = PSU;

    fn notify_consumer_disconnected(
        &mut self,
        psu: &'device Self::Psu,
        flags: ConsumerDisconnect,
    ) -> impl Future<Output = Result<(), crate::service::notification::Error>> {
        ready(
            self.sender
                .try_send(Event::ConsumerDisconnected(psu, flags))
                .ok_or(crate::service::notification::Error::WouldBlock),
        )
    }

    fn notify_consumer_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), crate::service::notification::Error>> {
        ready(
            self.sender
                .try_send(Event::ConsumerConnected(psu, capability))
                .ok_or(crate::service::notification::Error::WouldBlock),
        )
    }

    fn notify_provider_disconnected(
        &mut self,
        psu: &'device Self::Psu,
    ) -> impl Future<Output = Result<(), crate::service::notification::Error>> {
        ready(
            self.sender
                .try_send(Event::ProviderDisconnected(psu))
                .ok_or(crate::service::notification::Error::WouldBlock),
        )
    }

    fn notify_provider_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ProviderPowerCapability,
    ) -> impl Future<Output = Result<(), crate::service::notification::Error>> {
        ready(
            self.sender
                .try_send(Event::ProviderConnected(psu, capability))
                .ok_or(crate::service::notification::Error::WouldBlock),
        )
    }

    fn notify_unconstrained(
        &mut self,
        unconstrained: UnconstrainedState,
    ) -> impl Future<Output = Result<(), crate::service::notification::Error>> {
        ready(
            self.sender
                .try_send(Event::Unconstrained(unconstrained))
                .ok_or(crate::service::notification::Error::WouldBlock),
        )
    }
}

impl<'device, PSU: Lockable<Inner: Psu>, S: NonBlockingSender<Event<'device, PSU>>> From<S>
    for NonBlockingSenderNotifier<'device, PSU, S>
{
    fn from(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}

/// New-type that implements the [`crate::service::notification::Notifier`] trait for any [`Sender<Event>`].
///
/// This allows the user to choose blocking/non-blocking behavior when a type supports both.
pub struct SenderNotifier<'device, PSU: Lockable<Inner: Psu> + 'device, S: Sender<Event<'device, PSU>>> {
    pub sender: S,
    _phantom: PhantomData<&'device PSU>,
}

impl<'device, PSU: Lockable<Inner: Psu> + 'device, S: Sender<Event<'device, PSU>>> SenderNotifier<'device, PSU, S> {
    /// Create a new [`SenderNotifier`]
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}

impl<'device, PSU: Lockable<Inner: Psu> + 'device, S: Sender<Event<'device, PSU>>>
    crate::service::notification::Notifier<'device> for SenderNotifier<'device, PSU, S>
{
    type Psu = PSU;

    async fn notify_consumer_disconnected(
        &mut self,
        psu: &'device Self::Psu,
        flags: ConsumerDisconnect,
    ) -> Result<(), crate::service::notification::Error> {
        self.sender.send(Event::ConsumerDisconnected(psu, flags)).await;
        Ok(())
    }

    async fn notify_consumer_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ConsumerPowerCapability,
    ) -> Result<(), crate::service::notification::Error> {
        self.sender.send(Event::ConsumerConnected(psu, capability)).await;
        Ok(())
    }

    async fn notify_provider_disconnected(
        &mut self,
        psu: &'device Self::Psu,
    ) -> Result<(), crate::service::notification::Error> {
        self.sender.send(Event::ProviderDisconnected(psu)).await;
        Ok(())
    }

    async fn notify_provider_connected(
        &mut self,
        psu: &'device Self::Psu,
        capability: ProviderPowerCapability,
    ) -> Result<(), crate::service::notification::Error> {
        self.sender.send(Event::ProviderConnected(psu, capability)).await;
        Ok(())
    }

    async fn notify_unconstrained(
        &mut self,
        unconstrained: UnconstrainedState,
    ) -> Result<(), crate::service::notification::Error> {
        self.sender.send(Event::Unconstrained(unconstrained)).await;
        Ok(())
    }
}

impl<'device, PSU: Lockable<Inner: Psu>, S: Sender<Event<'device, PSU>>> From<S> for SenderNotifier<'device, PSU, S> {
    fn from(sender: S) -> Self {
        Self {
            sender,
            _phantom: PhantomData,
        }
    }
}
