//! Top level activity service generics and interface

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

use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
// use OnceLock for pub-sub interface allocation
use embassy_sync::once_lock::OnceLock;
// backend is PubSubChannel
use embassy_sync::pubsub::{Error, PubSubChannel, Publisher, Subscriber};

const MAX_SUBSCRIBERS: usize = 64; // TODO

struct Manager {
    chn: PubSubChannel<NoopRawMutex, Notification, 1, MAX_SUBSCRIBERS, 1>,
    keyboard_publisher_registered: Cell<bool>,
    trackpad_publisher_registered: Cell<bool>,
}

/// Opaque handle to publisher, cannot be constructed outside of register_publisher()
pub struct PublisherHandle(Class);

/// Opaque subscriber handle, disallow unapproved subscription methods for this service
pub struct SubscriberHandle(Subscriber<'static, NoopRawMutex, Notification, 1, MAX_SUBSCRIBERS, 1>);

static MANAGER: OnceLock<Manager> = OnceLock::new();
static PUBLISHER: OnceLock<Publisher<'static, NoopRawMutex, Notification, 1, MAX_SUBSCRIBERS, 1>> = OnceLock::new();

/// generate a subscriber handle
pub async fn subscribe() -> Result<SubscriberHandle, Error> {
    let subscribe_attempt = MANAGER.get().await.chn.subscriber();

    if subscribe_attempt.is_err() {
        Err(subscribe_attempt.err().unwrap())
    } else {
        Ok(SubscriberHandle(subscribe_attempt.unwrap()))
    }
}

/// wait for the next activity event broadcast
pub async fn wait(subscriber: &mut SubscriberHandle) -> Notification {
    subscriber.0.next_message_pure().await
}

/// attempt to register a publisher for Activity Class class. None if there is already a publisher registered for this class
pub async fn register_publisher(class: Class) -> Option<PublisherHandle> {
    match class {
        // only allow one registered keyboard activity publisher
        Class::Keyboard => {
            let manager = MANAGER.get().await;

            if manager.keyboard_publisher_registered.get() {
                None
            } else {
                manager.keyboard_publisher_registered.set(true);
                Some(PublisherHandle(class))
            }
        }

        // only allow one registered trackpad activity publisher
        Class::Trackpad => {
            let manager = MANAGER.get().await;

            if manager.trackpad_publisher_registered.get() {
                None
            } else {
                manager.trackpad_publisher_registered.set(true);
                Some(PublisherHandle(class))
            }
        }

        // allow any number of OEM activity publishers
        Class::Oem(_tag) => Some(PublisherHandle(class)),
    }
}

/// allow a publisher to transmit data to any current subscribers
/// API is only accessible if the publisher handle has been constructed validly via registration above
pub async fn publish(handle: &PublisherHandle, state: State) {
    PUBLISHER
        .get()
        .await
        .publish(Notification {
            state: state,
            class: handle.0,
        })
        .await;
}

/// initialize service resources
pub(crate) fn init() {
    let man = MANAGER.get_or_init(|| Manager {
        chn: PubSubChannel::new(),
        keyboard_publisher_registered: Cell::new(false),
        trackpad_publisher_registered: Cell::new(false),
    });

    PUBLISHER.get_or_init(|| man.chn.publisher().unwrap());
}
