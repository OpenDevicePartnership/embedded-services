//! Common traits for event senders and receivers

use embassy_sync::channel::{DynamicReceiver, DynamicSender};

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
