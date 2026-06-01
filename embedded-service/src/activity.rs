//! Activity Service Definitions

use embassy_sync::once_lock::OnceLock;

use crate::{SyncCell, intrusive_list::*, trace};

/// potential activity service states
#[derive(Copy, Clone, Debug)]
pub enum State {
    /// the service is currently active
    Active,

    /// the service is currently in-active, but could become active
    Inactive,

    /// the service is disabled and will not become active
    Disabled,
}

/// specifies OEM identifier for extended activity services
pub type OemIdentifier = u32;

/// specifies which Activity Class is updating state
#[derive(Copy, Clone, Debug)]
pub enum Class {
    /// the keyboard, if present, is currently active (keys pressed), inactive (keys released), or disabled (key scanning disabled)
    Keyboard,

    /// the trackpad, if present, is currently active (swiped), inactive (no swiped), or disabled (powered off/unavailable)
    Trackpad,

    // SecureUpdate, others as needed for ec template
    /// OEM Extension class, for activity notifications that are OEM specific
    Oem(OemIdentifier),
}

/// notification datagram, containing who's activity state (class) changed and what the new state is
#[derive(Copy, Clone, Debug)]
pub struct Notification {
    /// activity state of this class
    pub state: State,

    /// classification of activity
    pub class: Class,
}

/// trait to be implemented by any Activity service subscribers
pub trait ActivitySubscriber {
    /// async function invoked when Activity service update occurs
    fn activity_update(&self, notif: &Notification);
}

/// actual subscriber node instance for embedding within static or singleton type T
pub struct Subscriber {
    node: Node,
    instance: SyncCell<Option<&'static dyn ActivitySubscriber>>,
}

impl Subscriber {
    /// use this when static initialization occurs, internal fields will be validated in register_subscriber() later
    pub const fn uninit() -> Self {
        Self {
            node: Node::uninit(),
            instance: SyncCell::new(None),
        }
    }

    /// initializes the internal representation of this container's Activity Subscriber node
    fn init<T: ActivitySubscriber>(&self, container: &'static T) {
        self.instance.set(Some(container));
    }

    /// generates internal update over initialized data
    fn update(&self, notif: &Notification) {
        if let Some(subscriber) = self.instance.get() {
            subscriber.activity_update(notif);
        }
    }
}

impl NodeContainer for Subscriber {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

/// Publisher handle for registered publishers
#[derive(Copy, Clone, Debug)]
pub struct Publisher {
    class: Class,
}

/// register your subscriber to begin receiving updates
pub async fn register_subscriber<T: ActivitySubscriber>(
    this: &'static T,
    subscriber: &'static Subscriber,
) -> Result<()> {
    subscriber.init(this);
    SUBSCRIBERS.get().await.push(subscriber)
}

/// register publisher class for future usage. None returned if class slot is already occupied
pub fn register_publisher(class: Class) -> core::result::Result<Publisher, core::convert::Infallible> {
    // allow multiple publishers for any class (todo - determine if limitation is necessary)
    Ok(Publisher { class })
}

impl Publisher {
    /// publish state update
    pub async fn publish(&self, state: State) {
        let subs = SUBSCRIBERS.get().await;

        // build publisher-side "queue" of outbound messages
        let notif = Notification {
            state,
            class: self.class,
        };

        // note: this queue publication order can later be dispatched according to priorities if using a
        // single-executor that allows task level prioritization of futures.

        for listener_node in subs {
            // Skip-with-trace if the list ever holds a non-`Subscriber`
            // `NodeContainer`. The public registration API only accepts
            // `Subscriber`, but any crate-local code that pushes another
            // `NodeContainer` type to the same list would otherwise crash
            // publication (an `assert!` here would panic).
            let Some(subscriber) = listener_node.data::<Subscriber>() else {
                trace!("activity: skipping non-Subscriber node in SUBSCRIBERS list");
                continue;
            };
            subscriber.update(&notif);
        }
    }
}

static SUBSCRIBERS: OnceLock<IntrusiveList> = OnceLock::new();

pub(crate) fn init() {
    SUBSCRIBERS.get_or_init(IntrusiveList::new);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use embassy_sync::once_lock::OnceLock as TestOnceLock;

    /// A no-op `NodeContainer` that is intentionally NOT a `Subscriber`.
    ///
    /// Used to construct the pathological case where `SUBSCRIBERS` contains a
    /// `NodeContainer` whose downcast to `Subscriber` fails. A pre-fix
    /// `assert!(instance.is_some())` in `Publisher::publish` panicked here.
    struct ForeignContainer {
        node: Node,
    }

    impl NodeContainer for ForeignContainer {
        fn get_node(&self) -> &Node {
            &self.node
        }
    }

    /// A counting subscriber to confirm the publish loop still dispatches to
    /// valid subscribers even when foreign containers are mixed in.
    struct CountingSubscriber {
        node: Subscriber,
        hits: AtomicU32,
    }

    impl ActivitySubscriber for CountingSubscriber {
        fn activity_update(&self, _notif: &Notification) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// `Publisher::publish` must not panic when the global `SUBSCRIBERS` list
    /// holds a `NodeContainer` whose `data::<Subscriber>()` downcast returns
    /// `None`. A naive `assert!(instance.is_some())` here would be a hidden
    /// runtime hazard (the workspace `panic = "deny"` lint does not catch
    /// `assert!`).
    ///
    /// This test directly seeds `SUBSCRIBERS` with a non-`Subscriber`
    /// `NodeContainer` (only possible from within the crate) plus one real
    /// subscriber, then publishes. The publish must:
    ///   - return without panicking,
    ///   - still dispatch the notification to the real subscriber.
    #[tokio::test]
    async fn test_publish_skips_foreign_node_container_without_panicking() {
        // Single global init guard so this test is independent of other tests
        // touching SUBSCRIBERS.
        static INIT_DONE: AtomicBool = AtomicBool::new(false);
        SUBSCRIBERS.get_or_init(IntrusiveList::new);

        // Push a foreign NodeContainer that is NOT a Subscriber. This is the
        // pathological condition the old assert reacted to.
        static FOREIGN: TestOnceLock<ForeignContainer> = TestOnceLock::new();
        let foreign = FOREIGN.get_or_init(|| ForeignContainer { node: Node::uninit() });

        // Push a real subscriber.
        static REAL: TestOnceLock<CountingSubscriber> = TestOnceLock::new();
        let real = REAL.get_or_init(|| CountingSubscriber {
            node: Subscriber::uninit(),
            hits: AtomicU32::new(0),
        });

        // First-time init: register both. Subsequent test runs are no-ops
        // (the OnceLocks are global static; cargo runs each test once per process).
        if !INIT_DONE.swap(true, Ordering::SeqCst) {
            SUBSCRIBERS.get().await.push(foreign).unwrap();
            real.node.init(real);
            SUBSCRIBERS.get().await.push(&real.node).unwrap();
        }

        let publisher = register_publisher(Class::Keyboard).unwrap();

        // Snapshot hits before/after to be robust against test ordering.
        let before = real.hits.load(Ordering::SeqCst);
        // The pre-fix code panics here when iterating into the foreign container.
        publisher.publish(State::Active).await;
        let after = real.hits.load(Ordering::SeqCst);

        assert!(after > before, "real subscriber must still be dispatched to");
    }
}
