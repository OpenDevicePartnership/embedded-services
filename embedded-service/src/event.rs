//! Common traits for event senders and receivers
use core::marker::PhantomData;

use crate::error;

use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    channel::{DynamicReceiver, DynamicSender, Receiver as ChannelReceiver, Sender as ChannelSender},
    pubsub::{DynImmediatePublisher, DynPublisher, DynSubscriber, WaitResult},
};

/// Common event sender trait
pub trait NonBlockingSender<E> {
    /// Attempt to send an event
    ///
    /// Return none if sending the event would block.
    fn try_send(&mut self, event: E) -> Option<()>;
}

/// A sender that can block
pub trait Sender<E>: NonBlockingSender<E> {
    /// Send an event
    ///
    /// This blocks if the event cannot be sent immediately.
    fn send(&mut self, event: E) -> impl Future<Output = ()>;
}

/// Common event receiver trait
pub trait Receiver<E> {
    /// Attempt to receive an event
    ///
    /// Return none if there are no pending events
    fn try_next(&mut self) -> Option<E>;
    /// Receive an event
    fn wait_next(&mut self) -> impl Future<Output = E>;
}

/// Enum for receivers that can receive immediate events
#[derive(Clone)]
pub enum ImmediateEvent<E> {
    /// Event
    Event(E),
    /// Lagged events
    Lagged(u64),
}

impl<E> NonBlockingSender<E> for DynamicSender<'_, E> {
    fn try_send(&mut self, event: E) -> Option<()> {
        DynamicSender::try_send(self, event).ok()
    }
}

impl<E> Sender<E> for DynamicSender<'_, E> {
    fn send(&mut self, event: E) -> impl Future<Output = ()> {
        DynamicSender::send(self, event)
    }
}

impl<E> Receiver<E> for DynamicReceiver<'_, E> {
    fn try_next(&mut self) -> Option<E> {
        self.try_receive().ok()
    }

    fn wait_next(&mut self) -> impl Future<Output = E> {
        self.receive()
    }
}

impl<E: Clone> NonBlockingSender<E> for DynImmediatePublisher<'_, E> {
    fn try_send(&mut self, event: E) -> Option<()> {
        self.publish_immediate(event);
        Some(())
    }
}

impl<E: Clone> NonBlockingSender<E> for DynPublisher<'_, E> {
    fn try_send(&mut self, event: E) -> Option<()> {
        self.try_publish(event).ok()
    }
}

impl<E: Clone> Sender<E> for DynPublisher<'_, E> {
    fn send(&mut self, event: E) -> impl Future<Output = ()> {
        self.publish(event)
    }
}

impl<E: Clone> Receiver<E> for DynSubscriber<'_, E> {
    fn try_next(&mut self) -> Option<E> {
        match self.try_next_message() {
            Some(WaitResult::Message(e)) => Some(e),
            Some(WaitResult::Lagged(e)) => {
                error!("Subscriber lagged, skipping {} events", e);
                None
            }
            _ => None,
        }
    }

    async fn wait_next(&mut self) -> E {
        loop {
            match self.next_message().await {
                WaitResult::Message(e) => return e,
                WaitResult::Lagged(e) => {
                    error!("Subscriber lagged, skipping {} events", e);
                    continue;
                }
            }
        }
    }
}

impl<E: Clone> Receiver<ImmediateEvent<E>> for DynSubscriber<'_, E> {
    fn try_next(&mut self) -> Option<ImmediateEvent<E>> {
        match self.try_next_message() {
            Some(WaitResult::Message(e)) => Some(ImmediateEvent::Event(e)),
            Some(WaitResult::Lagged(e)) => Some(ImmediateEvent::Lagged(e)),
            _ => None,
        }
    }

    async fn wait_next(&mut self) -> ImmediateEvent<E> {
        match self.next_message().await {
            WaitResult::Message(e) => ImmediateEvent::Event(e),
            WaitResult::Lagged(e) => ImmediateEvent::Lagged(e),
        }
    }
}

impl<M: RawMutex, E, const N: usize> NonBlockingSender<E> for ChannelSender<'_, M, E, N> {
    fn try_send(&mut self, event: E) -> Option<()> {
        ChannelSender::try_send(self, event).ok()
    }
}

impl<M: RawMutex, E, const N: usize> Sender<E> for ChannelSender<'_, M, E, N> {
    fn send(&mut self, event: E) -> impl Future<Output = ()> {
        ChannelSender::send(self, event)
    }
}

impl<M: RawMutex, E, const N: usize> Receiver<E> for ChannelReceiver<'_, M, E, N> {
    fn try_next(&mut self) -> Option<E> {
        ChannelReceiver::try_receive(self).ok()
    }

    fn wait_next(&mut self) -> impl Future<Output = E> {
        ChannelReceiver::receive(self)
    }
}

/// A sender that discards all events sent to it.
pub struct NoopSender;

impl<E> NonBlockingSender<E> for NoopSender {
    fn try_send(&mut self, _event: E) -> Option<()> {
        Some(())
    }
}

impl<E> Sender<E> for NoopSender {
    async fn send(&mut self, _event: E) {}
}

/// Applies a function on events before passing them to the wrapped sender
pub struct MapSender<I, O, S: NonBlockingSender<O>, F: FnMut(I) -> O> {
    sender: S,
    map_fn: F,
    _phantom: PhantomData<(I, O)>,
}

impl<I, O, S: NonBlockingSender<O>, F: FnMut(I) -> O> MapSender<I, O, S, F> {
    /// Create a new self
    pub fn new(sender: S, map_fn: F) -> Self {
        Self {
            sender,
            map_fn,
            _phantom: PhantomData,
        }
    }
}

impl<I, O, S: NonBlockingSender<O>, F: FnMut(I) -> O> NonBlockingSender<I> for MapSender<I, O, S, F> {
    fn try_send(&mut self, event: I) -> Option<()> {
        self.sender.try_send((self.map_fn)(event))
    }
}

impl<I, O, S: Sender<O>, F: FnMut(I) -> O> Sender<I> for MapSender<I, O, S, F> {
    fn send(&mut self, event: I) -> impl Future<Output = ()> {
        self.sender.send((self.map_fn)(event))
    }
}

/// Applies a function on events received from the wrapped receiver.
pub struct MapReceiver<I, O, R: Receiver<I>, F: FnMut(I) -> O> {
    receiver: R,
    map_fn: F,
    _phantom: PhantomData<(I, O)>,
}

impl<I, O, R: Receiver<I>, F: FnMut(I) -> O> MapReceiver<I, O, R, F> {
    /// Create a new MapReceiver.
    pub fn new(receiver: R, map_fn: F) -> Self {
        Self {
            receiver,
            map_fn,
            _phantom: PhantomData,
        }
    }
}

impl<I, O, R: Receiver<I>, F: FnMut(I) -> O> Receiver<O> for MapReceiver<I, O, R, F> {
    fn try_next(&mut self) -> Option<O> {
        self.receiver.try_next().map(&mut self.map_fn)
    }

    async fn wait_next(&mut self) -> O {
        (self.map_fn)(self.receiver.wait_next().await)
    }
}

/// Filters events from the wrapped receiver, only yielding events that pass the predicate.
///
/// Events that do not pass the filter are consumed and discarded.
pub struct FilterReceiver<E, R: Receiver<E>, F: FnMut(&E) -> bool> {
    receiver: R,
    filter_fn: F,
    _phantom: PhantomData<E>,
}

impl<E, R: Receiver<E>, F: FnMut(&E) -> bool> FilterReceiver<E, R, F> {
    /// Create a new FilterReceiver.
    pub fn new(receiver: R, filter_fn: F) -> Self {
        Self {
            receiver,
            filter_fn,
            _phantom: PhantomData,
        }
    }
}

impl<E, R: Receiver<E>, F: FnMut(&E) -> bool> Receiver<E> for FilterReceiver<E, R, F> {
    fn try_next(&mut self) -> Option<E> {
        loop {
            match self.receiver.try_next() {
                Some(e) if (self.filter_fn)(&e) => return Some(e),
                Some(_) => continue,
                None => return None,
            }
        }
    }

    async fn wait_next(&mut self) -> E {
        loop {
            let e = self.receiver.wait_next().await;
            if (self.filter_fn)(&e) {
                return e;
            }
        }
    }
}

/// A receiver that never produces events.
///
/// This is mainly used to make it easier to construct a `MuxReceiver`
/// via macro since we don't need to handle the special start case
/// when chaining `with` calls.
pub struct NeverReceiver<E>(PhantomData<E>);

impl<E> NeverReceiver<E> {
    /// Create a new NeverReceiver.
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E> Default for NeverReceiver<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Receiver<E> for NeverReceiver<E> {
    fn try_next(&mut self) -> Option<E> {
        None
    }

    async fn wait_next(&mut self) -> E {
        core::future::pending().await
    }
}

/// Combines multiple receivers into one by racing them and returning
/// the first event that becomes available mapped to a common event type.
pub struct MuxReceiver<E, L: Receiver<E>, R: Receiver<E>> {
    left: L,
    right: R,
    _phantom: PhantomData<E>,
}

impl<E> MuxReceiver<E, NeverReceiver<E>, NeverReceiver<E>> {
    /// Create an empty MuxReceiver.
    ///
    /// Use `.with()` to add receivers.
    pub fn new() -> Self {
        Self {
            left: NeverReceiver::new(),
            right: NeverReceiver::new(),
            _phantom: PhantomData,
        }
    }
}

impl<E> Default for MuxReceiver<E, NeverReceiver<E>, NeverReceiver<E>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E, L: Receiver<E>, R1: Receiver<E>> MuxReceiver<E, L, R1> {
    /// Add another receiver to multiplex with this one.
    pub fn with<I, R2: Receiver<I>, F: FnMut(I) -> E>(
        self,
        receiver: R2,
        map_fn: F,
    ) -> MuxReceiver<E, Self, MapReceiver<I, E, R2, F>> {
        MuxReceiver {
            left: self,
            right: MapReceiver::new(receiver, map_fn),
            _phantom: PhantomData,
        }
    }
}

impl<E, L: Receiver<E>, R: Receiver<E>> Receiver<E> for MuxReceiver<E, L, R> {
    fn try_next(&mut self) -> Option<E> {
        self.left.try_next().or_else(|| self.right.try_next())
    }

    async fn wait_next(&mut self) -> E {
        match embassy_futures::select::select(self.left.wait_next(), self.right.wait_next()).await {
            embassy_futures::select::Either::First(e) => e,
            embassy_futures::select::Either::Second(e) => e,
        }
    }
}
