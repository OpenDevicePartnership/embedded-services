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
                crate::metrics::broadcaster::bump_lag(e);
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
                    crate::metrics::broadcaster::bump_lag(e);
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
            Some(WaitResult::Lagged(e)) => {
                crate::metrics::broadcaster::bump_lag(e);
                Some(ImmediateEvent::Lagged(e))
            }
            _ => None,
        }
    }

    async fn wait_next(&mut self) -> ImmediateEvent<E> {
        match self.next_message().await {
            WaitResult::Message(e) => ImmediateEvent::Event(e),
            WaitResult::Lagged(e) => {
                crate::metrics::broadcaster::bump_lag(e);
                ImmediateEvent::Lagged(e)
            }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    extern crate std;
    use super::*;
    use crate::GlobalRawMutex;
    use embassy_sync::pubsub::{PubSubChannel, Publisher};
    use static_cell::StaticCell;

    /// `DynSubscriber`'s `Receiver<E>::try_next` impl must bump the
    /// `metrics::broadcaster::lag` counter when it observes a
    /// `WaitResult::Lagged(n)`. This is the wrapper path used by
    /// consumers; the raw `next_message` API does not bump (it's
    /// embassy-sync, outside our wrapper).
    #[tokio::test]
    async fn test_dyn_subscriber_lag_bumps_counter() {
        static CHANNEL: StaticCell<PubSubChannel<GlobalRawMutex, u32, 1, 1, 1>> = StaticCell::new();
        let channel = CHANNEL.init(PubSubChannel::new());

        let publisher: Publisher<'_, GlobalRawMutex, u32, 1, 1, 1> = channel.publisher().unwrap();
        let mut subscriber = channel.dyn_subscriber().unwrap();

        // Lag the subscriber: push 2 messages into a 1-deep channel without
        // ever reading. The second push displaces the first.
        publisher.publish_immediate(1);
        publisher.publish_immediate(2);

        let before = crate::metrics::broadcaster::lag();

        // First wrapper read observes the lag. Our event::Receiver impl
        // returns None (try_next) or loops past (wait_next) after bumping.
        // try_next is synchronous, so use that.
        let result: Option<u32> = <DynSubscriber<u32> as Receiver<u32>>::try_next(&mut subscriber);
        // The first call returned None because it consumed the Lagged event.
        assert!(result.is_none(), "first try_next on lagged subscriber must return None");

        let after = crate::metrics::broadcaster::lag();
        assert!(
            after > before,
            "broadcaster::lag must increase after a lagged DynSubscriber read; before={} after={}",
            before,
            after,
        );

        // The next try_next should return the actual queued message (2),
        // and must NOT bump the lag counter again.
        let lag_after_first = crate::metrics::broadcaster::lag();
        let actual: Option<u32> = <DynSubscriber<u32> as Receiver<u32>>::try_next(&mut subscriber);
        assert_eq!(actual, Some(2), "after consuming lag, the next read returns the queued value");
        let lag_after_second = crate::metrics::broadcaster::lag();
        assert_eq!(
            lag_after_second, lag_after_first,
            "lag counter must not bump on a successful (non-lagged) read"
        );
    }
}
