//! Common traits for event senders and receivers
use core::{future::ready, marker::PhantomData};

use crate::error;

use embassy_sync::{
    channel::{DynamicReceiver, DynamicSender},
    pubsub::{DynImmediatePublisher, DynSubscriber, WaitResult},
};

/// Common event sender trait
pub trait Sender<E> {
    /// Attempt to send an event
    ///
    /// Return none if the event cannot currently be sent
    fn try_send(&mut self, event: E) -> Option<()>;
    /// Send an event
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

impl<E> Sender<E> for DynamicSender<'_, E> {
    fn try_send(&mut self, event: E) -> Option<()> {
        DynamicSender::try_send(self, event).ok()
    }

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

impl<E: Clone> Sender<E> for DynImmediatePublisher<'_, E> {
    fn try_send(&mut self, event: E) -> Option<()> {
        self.try_publish(event).ok()
    }

    fn send(&mut self, event: E) -> impl Future<Output = ()> {
        self.publish_immediate(event);
        ready(())
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
            if let WaitResult::Message(e) = self.next_message().await {
                return e;
            }
        }
    }
}

/// A sender that discards all events sent to it.
pub struct NoopSender;

impl<E> Sender<E> for NoopSender {
    fn try_send(&mut self, _event: E) -> Option<()> {
        Some(())
    }

    async fn send(&mut self, _event: E) {}
}

/// Applies a function on events before passing them to the wrapped sender
pub struct MapSender<I, O, S: Sender<O>, F: FnMut(I) -> O> {
    sender: S,
    map_fn: F,
    _phantom: PhantomData<(I, O)>,
}

impl<I, O, S: Sender<O>, F: FnMut(I) -> O> MapSender<I, O, S, F> {
    /// Create a new self
    pub fn new(sender: S, map_fn: F) -> Self {
        Self {
            sender,
            map_fn,
            _phantom: PhantomData,
        }
    }
}

impl<I, O, S: Sender<O>, F: FnMut(I) -> O> Sender<I> for MapSender<I, O, S, F> {
    fn try_send(&mut self, event: I) -> Option<()> {
        self.sender.try_send((self.map_fn)(event))
    }

    fn send(&mut self, event: I) -> impl Future<Output = ()> {
        self.sender.send((self.map_fn)(event))
    }
}
