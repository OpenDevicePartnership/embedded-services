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

// SAFETY: `DynImmediatePublisher<'a, T>` wraps `&dyn PubSubBehavior<T>`,
// a trait object that upstream embassy-sync does not bound with
// `Send + Sync`. The underlying `PubSubChannel` is itself internally
// synchronized via its own `RawMutex`, so the trait-object erasure is the
// only thing hiding `Send + Sync` from the type system. Under the single
// Cortex-M / single Embassy executor model documented in `lib.rs`, no
// receiver is ever actually moved or shared across an OS thread boundary
// in production; the manual impls below restore the bounds required by
// the tightened `NodeContainer: Send + Sync` contract in
// `intrusive_list.rs`.
unsafe impl<T: Clone + Send> Send for Receiver<'static, T> {}
// SAFETY: see the `Send` impl above. Internal synchronization is provided
// by the wrapped `Mutex<GlobalRawMutex, _>` plus the channel's own
// `RawMutex`; only the trait-object erasure of `PubSubBehavior<T>` hides
// that fact from the auto-derive.
unsafe impl<T: Clone + Send> Sync for Receiver<'static, T> {}

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

impl<T: Clone + Send> intrusive_list::NodeContainer for Receiver<'static, T> {
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

impl<T: Clone + Send + 'static> Immediate<T> {
    /// Register a receiver
    pub fn register_receiver(&self, receiver: &'static Receiver<'_, T>) -> intrusive_list::Result<()> {
        self.receivers.push(receiver)
    }

    /// Broadcast a message to all receivers
    ///
    /// # Cancel safety
    ///
    /// This future is **NOT cancel-safe**. The implementation iterates
    /// receivers and `.await`s a per-receiver mutex lock for each one. If the
    /// returned future is dropped between two iterations (e.g. by a `select`
    /// arm completing first, by `tokio::time::timeout`, or by task abort),
    /// **partial delivery** occurs: receivers earlier in the list have
    /// observed the message, those later in the list have not. There is no
    /// re-try, no log, and no signal to the caller about which receivers
    /// missed the message.
    ///
    /// In practice this is acceptable for the documented "messages may be
    /// lost" contract on this module, but callers performing time-critical
    /// broadcasts should avoid wrapping `broadcast` in cancellation
    /// combinators. A `try_broadcast` returning per-receiver status is a
    /// future broadcaster-redesign item.
    pub async fn broadcast(&self, message: T) {
        for node in &self.receivers {
            if let Some(receiver) = node.data::<Receiver<'_, T>>() {
                receiver.publisher.lock().await.publish_immediate(message.clone());
            }
        }
    }
}

/// `Receiver<'static, T>` must implement `Send + Sync` when `T: Clone + Send`,
/// per the manual `unsafe impl` blocks above. The asserts below would fail
/// to compile if the impls were ever removed.
///
/// ```
/// use embedded_services::broadcaster::immediate::Receiver;
/// fn assert_send_sync<T: Send + Sync>() {}
/// assert_send_sync::<Receiver<'static, u32>>();
/// assert_send_sync::<Receiver<'static, core::cell::Cell<u8>>>();
/// ```
///
/// A `!Send` payload (e.g. `*const u8`) must NOT yield a `Send + Sync`
/// `Receiver`, because the manual impl is gated on `T: Send`:
///
/// ```compile_fail
/// use embedded_services::broadcaster::immediate::Receiver;
/// fn assert_send_sync<T: Send + Sync>() {}
/// assert_send_sync::<Receiver<'static, *const u8>>();
/// ```
///
/// And a `!Send` payload must also not satisfy `NodeContainer`:
///
/// ```compile_fail
/// use embedded_services::broadcaster::immediate::Receiver;
/// use embedded_services::intrusive_list::NodeContainer;
/// fn assert_nc<T: NodeContainer>() {}
/// assert_nc::<Receiver<'static, *const u8>>();
/// ```
#[allow(dead_code)]
const _SEND_SYNC_DOCS: () = ();

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use embassy_sync::pubsub::{PubSubChannel, WaitResult};
    use static_cell::StaticCell;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn receiver_is_send_sync_for_send_payload() {
        // Compile-time assertion: the manual `unsafe impl Send/Sync` blocks
        // must continue to apply for `T: Send` payloads.
        assert_send_sync::<Receiver<'static, u32>>();
        assert_send_sync::<Receiver<'static, [u8; 16]>>();
        // `Cell<u8>: Send` (but `!Sync`), and our impl requires only `T: Send`,
        // so this must hold.
        assert_send_sync::<Receiver<'static, core::cell::Cell<u8>>>();
    }

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
