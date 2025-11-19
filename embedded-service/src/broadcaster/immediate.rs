//! Immediate broadcaster
//! No backpressure and unhandled messages may be lost if the subscriber's queue is full.

use core::marker::PhantomData;

use embassy_sync::{mutex::Mutex, pubsub::DynImmediatePublisher};

use crate::{GlobalRawMutex, intrusive_list};

/// Receiver
pub struct Receiver<'a, T: Clone> {
    node: intrusive_list::Node,
    publisher: Mutex<GlobalRawMutex, DynImmediatePublisher<'a, T>>,
}

impl<'a, T: Clone> Receiver<'a, T> {
    /// Create a new receiver
    pub fn new(publisher: DynImmediatePublisher<'a, T>) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            publisher: Mutex::new(publisher),
        }
    }
}

impl<'a, T: Clone> From<DynImmediatePublisher<'a, T>> for Receiver<'a, T> {
    fn from(publisher: DynImmediatePublisher<'a, T>) -> Self {
        Self::new(publisher)
    }
}

impl<T: Clone> intrusive_list::NodeContainer for Receiver<'static, T> {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

/// Immediate broadcaster
pub struct Immediate<T: Clone + 'static> {
    receivers: intrusive_list::IntrusiveList,
    _phantom: PhantomData<T>,
}

impl<T: Clone + 'static> Immediate<T> {
    /// Create a new `Immediate<T>`
    pub const fn new() -> Self {
        Self {
            receivers: intrusive_list::IntrusiveList::new(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Clone + 'static> Default for Immediate<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + 'static> Immediate<T> {
    /// Register a receiver
    pub fn register_receiver(&self, receiver: &'static Receiver<'_, T>) -> intrusive_list::Result<()> {
        self.receivers.push(receiver)
    }

    /// Broadcast a message to all receivers
    pub async fn broadcast(&self, message: T) {
        for node in &self.receivers {
            if let Some(receiver) = node.data::<Receiver<'_, T>>() {
                receiver.publisher.lock().await.publish_immediate(message.clone());
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use embassy_sync::pubsub::{PubSubChannel, WaitResult};
    use static_cell::StaticCell;

    /// Test normal functionality
    #[tokio::test]
    async fn test_immediate_broadcaster() {
        static CHANNEL: StaticCell<PubSubChannel<GlobalRawMutex, u32, 1, 1, 0>> = StaticCell::new();
        let channel = CHANNEL.init(PubSubChannel::new());

        let publisher = channel.dyn_immediate_publisher();
        let mut subscriber = channel.dyn_subscriber().unwrap();

        static RECEIVER: StaticCell<Receiver<'static, u32>> = StaticCell::new();
        let receiver = RECEIVER.init(Receiver::new(publisher));

        static BROADCASTER: StaticCell<Immediate<u32>> = StaticCell::new();
        let immediate_broadcaster = BROADCASTER.init(Immediate::default());

        immediate_broadcaster.register_receiver(receiver).unwrap();
        immediate_broadcaster.broadcast(42).await;

        let message = subscriber.next_message().await;
        assert_eq!(message, WaitResult::Message(42));
    }

    /// Test overflow
    #[tokio::test]
    async fn test_immediate_broadcaster_overflow() {
        static CHANNEL: StaticCell<PubSubChannel<GlobalRawMutex, u32, 1, 1, 0>> = StaticCell::new();
        let channel = CHANNEL.init(PubSubChannel::new());

        let publisher = channel.dyn_immediate_publisher();
        let mut subscriber = channel.dyn_subscriber().unwrap();

        static RECEIVER: StaticCell<Receiver<'static, u32>> = StaticCell::new();
        let receiver = RECEIVER.init(Receiver::new(publisher));

        static BROADCASTER: StaticCell<Immediate<u32>> = StaticCell::new();
        let immediate_broadcaster = BROADCASTER.init(Immediate::default());

        immediate_broadcaster.register_receiver(receiver).unwrap();
        immediate_broadcaster.broadcast(42).await;
        immediate_broadcaster.broadcast(34).await;

        let message = subscriber.next_message().await;
        assert_eq!(message, WaitResult::Lagged(1));

        let message = subscriber.next_message().await;
        assert_eq!(message, WaitResult::Message(34));
    }
}
