//! Activity Service Definitions

use embassy_sync::once_lock::OnceLock;

use crate::{intrusive_list::*, sync_cell::SyncCell};

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
pub async fn register_publisher(class: Class) -> core::result::Result<Publisher, core::convert::Infallible> {
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
            let instance = listener_node.data::<Subscriber>();
            // as subscriber list is only accessible via these safe interfaces, can perform an "invariant assert" here
            // to catch potential state or stack corruption later
            assert!(instance.is_some());

            if let Some(subscriber) = instance {
                subscriber.update(&notif);
            }
        }
    }
}

static SUBSCRIBERS: OnceLock<IntrusiveList> = OnceLock::new();

pub(crate) fn init() {
    SUBSCRIBERS.get_or_init(IntrusiveList::new);
}
