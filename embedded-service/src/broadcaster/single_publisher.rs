//! Single-publisher broadcast channel.
//!
//! A wrapper around [`embassy_sync::pubsub::PubSubChannel`] that enforces exactly one publisher
//! with any number of subscribers. The inner channel is hardcoded with `PUBS = 1`, so attempting
//! to create a second publisher returns [`embassy_sync::pubsub::Error::MaximumPublishersReached`].

use embassy_sync::pubsub::{self, DynPublisher, DynSubscriber, PubSubChannel};

use crate::GlobalRawMutex;

/// A broadcast channel that allows only a single publisher and multiple subscribers.
///
/// Wraps [`PubSubChannel`] with `PUBS` fixed to `1`. The returned [`DynPublisher`] supports
/// [`publish`](DynPublisher::publish), [`try_publish`](DynPublisher::try_publish), and
/// [`publish_immediate`](DynPublisher::publish_immediate).
pub struct SinglePublisherChannel<T: Clone, const CAP: usize, const SUBS: usize> {
    inner: PubSubChannel<GlobalRawMutex, T, CAP, SUBS, 1>,
}

impl<T: Clone, const CAP: usize, const SUBS: usize> SinglePublisherChannel<T, CAP, SUBS> {
    /// Create a new `SinglePublisherChannel`.
    pub const fn new() -> Self {
        Self {
            inner: PubSubChannel::new(),
        }
    }

    /// Obtain the single publisher for this channel.
    ///
    /// Returns [`pubsub::Error::MaximumPublishersReached`] if a publisher has already been created.
    pub fn publisher(&self) -> Result<DynPublisher<'_, T>, pubsub::Error> {
        self.inner.dyn_publisher()
    }

    /// Create a subscriber to this channel.
    ///
    /// Returns [`pubsub::Error::MaximumSubscribersReached`] if all subscriber slots are in use.
    pub fn subscriber(&self) -> Result<DynSubscriber<'_, T>, pubsub::Error> {
        self.inner.dyn_subscriber()
    }
}

impl<T: Clone, const CAP: usize, const SUBS: usize> Default for SinglePublisherChannel<T, CAP, SUBS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use embassy_sync::pubsub::WaitResult;
    use static_cell::StaticCell;

    /// Test that a single publisher can send a message received by a subscriber.
    #[tokio::test]
    async fn test_publish_and_receive() {
        static CHANNEL: StaticCell<SinglePublisherChannel<u32, 4, 1>> = StaticCell::new();
        let channel = CHANNEL.init(SinglePublisherChannel::new());

        let publisher = channel.publisher().unwrap();
        let mut subscriber = channel.subscriber().unwrap();

        publisher.publish_immediate(42);

        let message = subscriber.next_message().await;
        assert_eq!(message, WaitResult::Message(42));
    }

    /// Test that creating a second publisher returns an error.
    #[tokio::test]
    async fn test_second_publisher_rejected() {
        static CHANNEL: StaticCell<SinglePublisherChannel<u32, 4, 1>> = StaticCell::new();
        let channel = CHANNEL.init(SinglePublisherChannel::new());

        let _publisher = channel.publisher().unwrap();
        let result = channel.publisher();

        assert!(matches!(result, Err(pubsub::Error::MaximumPublishersReached)));
    }

    /// Test that multiple subscribers all receive the same broadcasted message.
    #[tokio::test]
    async fn test_multiple_subscribers() {
        static CHANNEL: StaticCell<SinglePublisherChannel<u32, 4, 3>> = StaticCell::new();
        let channel = CHANNEL.init(SinglePublisherChannel::new());

        let publisher = channel.publisher().unwrap();
        let mut sub1 = channel.subscriber().unwrap();
        let mut sub2 = channel.subscriber().unwrap();
        let mut sub3 = channel.subscriber().unwrap();

        publisher.publish_immediate(99);

        assert_eq!(sub1.next_message().await, WaitResult::Message(99));
        assert_eq!(sub2.next_message().await, WaitResult::Message(99));
        assert_eq!(sub3.next_message().await, WaitResult::Message(99));
    }
}
