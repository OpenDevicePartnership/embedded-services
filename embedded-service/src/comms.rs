//! Comms Service Definitions

use core::any::{Any, TypeId};
use core::convert::Infallible;

use embassy_sync::once_lock::OnceLock;
use serde::{Deserialize, Serialize};

use crate::IntrusiveList;
use crate::SyncCell;
use crate::intrusive_list::{self, Node, NodeContainer};

/// key type for OEM Endpoint declarations
pub type OemKey = isize;

/// Internal endpoints, by generalized name
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Internal {
    /// platform information service provider
    PlatformInfo,

    /// keyboard manager
    Keyboard,

    /// HID service provider
    Hid,

    /// Host manager and boot control
    HostBoot,

    /// Power manager for the system
    Power,

    /// USB-C service provider
    Usbc,

    /// Thermal service provider
    Thermal,

    /// Trackpad service provider
    Trackpad,

    /// Battery service provider
    Battery,

    /// NVM service provider
    Nonvol,

    /// Debug service provider
    Debug,

    /// Security service provider
    Security,

    /// OEM defined receiver
    Oem(OemKey),
}

/// External identifier for routing
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum External {
    /// route a message to the host (typ. SoC with HLOS)
    Host,

    /// route a message to debug probe or utility
    Debug,

    /// route a message to an OEM defined target
    Oem(OemKey),
}

/// Endpoint identifier for routing
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EndpointID {
    /// route to/from an internal source
    Internal(Internal),

    /// route to/from an external source
    External(External),
}

impl From<Internal> for EndpointID {
    fn from(value: Internal) -> Self {
        EndpointID::Internal(value)
    }
}

impl From<External> for EndpointID {
    fn from(value: External) -> Self {
        EndpointID::External(value)
    }
}

/// Data reference -- generalized such that any stack variable can be transmitted "in place" as needed
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Data<'a> {
    contents: &'a (dyn Any + Send + Sync),
}

impl<'a> Data<'a> {
    /// Construct a Data portion of a Message from some data input
    pub fn new(from: &'a (impl Any + Send + Sync)) -> Self {
        Self { contents: from }
    }

    /// Attempt to retrieve data as type T -- None if incorrect type
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.contents.downcast_ref()
    }

    /// Fetch type ID for message contents to allow reception of multiple top level elements
    /// Ex:
    /// ```
    /// # use core::any::TypeId;
    /// # use embedded_services::comms::{Data, Message, EndpointID, Internal};
    /// struct MessageClassA;
    /// struct MessageClassB;
    /// let message = Message {
    ///     from: EndpointID::from(Internal::PlatformInfo),
    ///     to: EndpointID::from(Internal::PlatformInfo),
    ///     data: Data::new(&MessageClassA),
    /// };
    /// if message.data.type_id() == TypeId::of::<MessageClassA>() {
    ///     // do something
    /// } else if message.data.type_id() == TypeId::of::<MessageClassB>() {
    ///     // do something else
    /// } else {
    ///     // do something else
    /// }
    /// ```
    pub fn type_id(&self) -> TypeId {
        self.contents.type_id()
    }

    /// Shorthand if only a few Message types are supported by an Endpoint:
    /// if `data.is_a::<MessageClassA>() {}`
    /// else if `data.is_a::<MessageClassB>() {}`
    /// etc.
    pub fn is_a<T: Any + Send + Sync>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }
}

/// Message to receive
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Message<'a> {
    /// where this message came from
    pub from: EndpointID,

    /// where this message is going
    pub to: EndpointID,

    /// message content
    pub data: Data<'a>,
}

/// Trait to receive messages
pub trait MailboxDelegate {
    /// Receive a Message (typically, push contents to queue or queue some action)
    fn receive(&self, _message: &Message) -> Result<(), MailboxDelegateError> {
        Ok(())
    }
}

/// Message transmission Error
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MailboxDelegateError {
    /// Buffer is full
    BufferFull,

    /// Message not found
    MessageNotFound,

    /// Invalid source
    InvalidSource,

    /// Invalid destination
    InvalidDestination,

    /// Invalid ID
    InvalidId,

    /// Invalid data
    InvalidData,

    /// Other error. Usually related to the underlying device or
    /// transport.
    Other,
}

/// Primary node registration for receiving messages from the comms service
pub struct Endpoint {
    node: Node,
    id: EndpointID,
    // NOTE: `Option<&'static dyn MailboxDelegate>` is `!Send` because the
    // `MailboxDelegate` trait has no `Sync` supertrait. We deliberately do
    // NOT add a `const _` `Send` guard here — see the parallel comment in
    // `activity.rs`. Soundness rests on the `unsafe impl Send + Sync for
    // Endpoint` below plus the single-executor model documented in
    // `lib.rs`.
    delegator: SyncCell<Option<&'static dyn MailboxDelegate>>,
}

impl NodeContainer for Endpoint {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

// SAFETY: The invariant: `Endpoint`'s sole non-`Send + Sync` state is the
// trait object stored inside `delegator: SyncCell<Option<&'static dyn
// MailboxDelegate>>`. `MailboxDelegate` is a public trait that does not
// require `Send + Sync` (changing it would break the public API), so the
// auto-derive of these markers fails purely on trait-object bound erasure
// — not on any actual sharing hazard in the storage. `&'static dyn Trait`
// is a fat pointer; sharing or sending the pointer itself is sound, and
// the `SyncCell` serializes all read/write of the slot under a critical
// section. Combined with the single Cortex-M / single Embassy executor
// model documented in `lib.rs`, no `Endpoint` is ever concurrently
// accessed by anything but the cooperatively-scheduled executor, so the
// manual impls restore the `Send + Sync` markers required by
// `NodeContainer: Send + Sync` in `intrusive_list.rs` without introducing
// any new sharing path.
unsafe impl Send for Endpoint {}
// SAFETY: same invariant as the `Send` impl above.
unsafe impl Sync for Endpoint {}

impl Endpoint {
    /// Get endpoint ID
    pub fn get_id(&self) -> EndpointID {
        self.id
    }

    /// use this when static initialization occurs, internal fields will be validated in register_subscriber() later
    pub const fn uninit(id: EndpointID) -> Self {
        Self {
            node: Node::uninit(),
            id,
            delegator: SyncCell::new(None),
        }
    }

    /// Send a generic message to an endpoint
    pub async fn send(&self, to: EndpointID, data: &(impl Any + Send + Sync)) -> Result<(), Infallible> {
        send(self.id, to, data).await
    }

    fn init(&self, rx: &'static dyn MailboxDelegate) {
        self.delegator.set(Some(rx));
    }

    fn process(&self, message: &Message) {
        if let Some(delegator) = self.delegator.get() {
            // Back-pressure policy: drop + warn-log + counter. Without these
            // a misbehaving / saturated delegate would silently lose messages.
            if let Err(err) = delegator.receive(message) {
                crate::warn!(
                    "comms: delegator returned {} for endpoint",
                    mailbox_delegate_error_str(err),
                );
                crate::metrics::comms::bump_delegator_errors();
            }
        } else {
            // The endpoint is registered (otherwise we would not be here) but
            // its delegator slot is still `None`. Under the single-executor
            // model this is impossible because `register_endpoint` runs
            // `push` and `init` with no yield in between; reaching this arm
            // therefore signals either a custom registration path that
            // bypassed `register_endpoint`, or a multi-executor consumer
            // that violates the documented model. Bumping the counter and
            // warn-logging keeps the silent-drop out of the no-failure
            // contract.
            crate::warn!("comms: routed message reached endpoint with uninitialized delegator");
            crate::metrics::comms::bump_delivered_to_uninit();
        }
    }
}

/// Stringify a [`MailboxDelegateError`] for backend-agnostic logging.
///
/// Both `defmt` and `log` accept `{}` against `&str`, so this is the lowest
/// common denominator that works on every supported logging backend without
/// requiring a `Display` impl on the error type.
const fn mailbox_delegate_error_str(err: MailboxDelegateError) -> &'static str {
    match err {
        MailboxDelegateError::BufferFull => "BufferFull",
        MailboxDelegateError::MessageNotFound => "MessageNotFound",
        MailboxDelegateError::InvalidSource => "InvalidSource",
        MailboxDelegateError::InvalidDestination => "InvalidDestination",
        MailboxDelegateError::InvalidId => "InvalidId",
        MailboxDelegateError::InvalidData => "InvalidData",
        MailboxDelegateError::Other => "Other",
    }
}

/// initialize receiver node for message handling
///
/// The list push happens BEFORE the delegator slot is written. If the same
/// node is already in the list (duplicate registration), the push returns
/// `Err(NodeAlreadyInList)` and the existing delegator is left intact —
/// reversing the order would silently overwrite the delegator while the push
/// rejected the duplicate. Under the single-executor assumption documented
/// in `lib.rs`, there is no yield point between the successful `push` and
/// the subsequent `init`, so the router cannot observe a
/// registered-but-uninitialized endpoint.
///
/// # Fan-out semantics
///
/// Registering N distinct `Endpoint`s under the same `EndpointID` is
/// allowed and produces fan-out delivery: every routed message addressed
/// to that id is dispatched to each registered delegate in turn (LIFO
/// iteration order). This is load-bearing for the existing test suite
/// and at least one in-tree consumer. Consumers that need
/// single-delegate semantics for a given id must enforce that themselves
/// (e.g. by checking via a sibling registry whether an endpoint already
/// exists, as `hid::register_device` does for `DeviceId`).
pub async fn register_endpoint(
    this: &'static impl MailboxDelegate,
    node: &'static Endpoint,
) -> Result<(), intrusive_list::Error> {
    get_list(node.id).get().await.push(node)?;
    node.init(this);
    Ok(())
}

fn get_list(target: EndpointID) -> &'static OnceLock<IntrusiveList> {
    match target {
        EndpointID::External(ext_endpoint) => match ext_endpoint {
            External::Host => {
                static EXTERNAL_HOST: OnceLock<IntrusiveList> = OnceLock::new();
                &EXTERNAL_HOST
            }
            External::Debug => {
                static EXTERNAL_DEBUG: OnceLock<IntrusiveList> = OnceLock::new();
                &EXTERNAL_DEBUG
            }
            External::Oem(_key) => {
                static EXTERNAL_OEM: OnceLock<IntrusiveList> = OnceLock::new();
                &EXTERNAL_OEM
            }
        },
        EndpointID::Internal(int_endpoint) => {
            use Internal::*;

            static INTERNAL_LIST_PLATFORM_INFO: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_KEYBOARD: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_HID: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_HOST_BOOT: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_POWER: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_USBC: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_THERMAL: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_TRACKPAD: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_BATTERY: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_NONVOL: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_DEBUG: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_SECURITY: OnceLock<IntrusiveList> = OnceLock::new();
            static INTERNAL_LIST_OEM: OnceLock<IntrusiveList> = OnceLock::new();

            match int_endpoint {
                PlatformInfo => &INTERNAL_LIST_PLATFORM_INFO,
                Keyboard => &INTERNAL_LIST_KEYBOARD,
                Hid => &INTERNAL_LIST_HID,
                HostBoot => &INTERNAL_LIST_HOST_BOOT,
                Power => &INTERNAL_LIST_POWER,
                Usbc => &INTERNAL_LIST_USBC,
                Thermal => &INTERNAL_LIST_THERMAL,
                Trackpad => &INTERNAL_LIST_TRACKPAD,
                Battery => &INTERNAL_LIST_BATTERY,
                Nonvol => &INTERNAL_LIST_NONVOL,
                Debug => &INTERNAL_LIST_DEBUG,
                Security => &INTERNAL_LIST_SECURITY,
                Oem(_key) => &INTERNAL_LIST_OEM,
            }
        }
    }
}

/// Send a generic message to an endpoint
pub async fn send(from: EndpointID, to: EndpointID, data: &(impl Any + Send + Sync)) -> Result<(), Infallible> {
    route(Message {
        from,
        to,
        data: Data::new(data),
    })
    .await
}

/// route a message to any valid receiver nodes
async fn route(message: Message<'_>) -> Result<(), Infallible> {
    let list = get_list(message.to).get().await;

    let mut delivered = 0usize;
    for rxq in list {
        if let Some(endpoint) = rxq.data::<Endpoint>()
            && message.to == endpoint.id
        {
            endpoint.process(&message);
            delivered = delivered.saturating_add(1);
        }
    }

    if delivered == 0 {
        // No endpoint matched. Could indicate a service that has not
        // registered yet, a routing-table init bug, or a typo in the
        // destination id. Counter is observable via
        // `metrics::comms::routed_unknown`.
        crate::metrics::comms::bump_routed_unknown();
    }

    Ok(())
}

pub(crate) fn init() {
    // initialize internal subscriber lists
    get_list(Internal::PlatformInfo.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Keyboard.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Hid.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::HostBoot.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Power.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Usbc.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Thermal.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Trackpad.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Battery.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Nonvol.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Debug.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Security.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Oem(0).into()).get_or_init(IntrusiveList::new);

    // initialize external subscriber lists
    get_list(External::Debug.into()).get_or_init(IntrusiveList::new);
    get_list(External::Host.into()).get_or_init(IntrusiveList::new);
    get_list(External::Oem(0).into()).get_or_init(IntrusiveList::new);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};
    use embassy_sync::once_lock::OnceLock;

    fn assert_send_sync<T: Send + Sync>() {}

    /// `Endpoint` carries a manual `unsafe impl Send + Sync`. This compile-time
    /// assertion guards against accidental removal of either impl.
    #[test]
    fn endpoint_is_send_sync() {
        assert_send_sync::<Endpoint>();
    }

    /// Move an `Endpoint` across a tokio worker thread boundary. This will
    /// fail to compile if `Endpoint: Send` regresses; at runtime it confirms
    /// the manual impl is not actively unsound on the host.
    #[tokio::test]
    async fn endpoint_crosses_thread_boundary() {
        let ep = Endpoint::uninit(EndpointID::Internal(Internal::PlatformInfo));
        let handle = tokio::spawn(async move {
            // Just read the id from the other thread.
            ep.get_id()
        });
        let id = handle.await.unwrap();
        assert!(matches!(id, EndpointID::Internal(Internal::PlatformInfo)));
    }

    /// A `MailboxDelegate` that counts received messages, so a test can prove
    /// which delegate handled a routed message.
    struct CountingDelegate {
        label: u32,
        hits: AtomicU32,
    }

    impl MailboxDelegate for CountingDelegate {
        fn receive(&self, _message: &Message) -> Result<(), MailboxDelegateError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A `MailboxDelegate` that ALWAYS rejects with `BufferFull` so we can
    /// prove (a) the rejection is reported via warn-log and (b) one failing
    /// delegate does not break delivery to others.
    struct RejectingDelegate {
        rejections: AtomicU32,
    }

    impl MailboxDelegate for RejectingDelegate {
        fn receive(&self, _message: &Message) -> Result<(), MailboxDelegateError> {
            self.rejections.fetch_add(1, Ordering::SeqCst);
            Err(MailboxDelegateError::BufferFull)
        }
    }

    /// `register_endpoint` must reject duplicate registration AND preserve
    /// the first delegator. Reversing the push/init order would silently
    /// overwrite the first delegator when the push rejected the duplicate.
    #[tokio::test]
    async fn test_register_endpoint_rejects_duplicate_and_preserves_first_delegator() {
        // Use the OEM external bucket to keep this test isolated from any
        // other internal/external endpoint state in the global lists.
        init();

        static FIRST: OnceLock<CountingDelegate> = OnceLock::new();
        let first = FIRST.get_or_init(|| CountingDelegate {
            label: 1,
            hits: AtomicU32::new(0),
        });
        static SECOND: OnceLock<CountingDelegate> = OnceLock::new();
        let second = SECOND.get_or_init(|| CountingDelegate {
            label: 2,
            hits: AtomicU32::new(0),
        });

        // One Endpoint, shared between two registration attempts.
        static ENDPOINT: OnceLock<Endpoint> = OnceLock::new();
        let endpoint = ENDPOINT.get_or_init(|| {
            // Pick an OEM key unlikely to collide with other tests.
            Endpoint::uninit(EndpointID::External(External::Oem(91)))
        });

        // First registration must succeed.
        register_endpoint(first, endpoint).await.unwrap();

        // Second registration with a different delegator must FAIL.
        let duplicate = register_endpoint(second, endpoint).await;
        assert!(
            matches!(duplicate, Err(intrusive_list::Error::NodeAlreadyInList)),
            "duplicate registration must return NodeAlreadyInList"
        );

        // Route a message; the original delegator (first) must handle it,
        // proving its delegator slot was preserved across the rejected
        // duplicate registration.
        let payload: u32 = 0xC0FFEE;
        let _ = endpoint.send(EndpointID::External(External::Oem(91)), &payload).await;

        assert_eq!(first.hits.load(Ordering::SeqCst), 1, "first delegator must receive");
        assert_eq!(
            second.hits.load(Ordering::SeqCst),
            0,
            "second delegator must NOT receive after rejected duplicate registration"
        );

        // Bonus: prove the labels (used purely for human debugging) are sane.
        assert_eq!(first.label, 1);
        assert_eq!(second.label, 2);
    }

    /// `Endpoint::process` must NOT swallow delegate errors silently. The
    /// error is reported via the `warn!` macro and the route continues to
    /// attempt delivery to other endpoints (one bad delegate must not break
    /// the rest).
    ///
    /// This test cannot easily capture log output without extra infrastructure,
    /// so it instead asserts behavior: with one rejecting + one accepting
    /// endpoint, the accepting one must still receive, and the rejecting one
    /// must record its rejection.
    #[tokio::test]
    async fn test_route_continues_past_rejecting_delegate() {
        init();

        static REJECTING: OnceLock<RejectingDelegate> = OnceLock::new();
        let rejecting = REJECTING.get_or_init(|| RejectingDelegate {
            rejections: AtomicU32::new(0),
        });
        static ACCEPTING: OnceLock<CountingDelegate> = OnceLock::new();
        let accepting = ACCEPTING.get_or_init(|| CountingDelegate {
            label: 99,
            hits: AtomicU32::new(0),
        });

        static REJECTING_ENDPOINT: OnceLock<Endpoint> = OnceLock::new();
        let rejecting_endpoint =
            REJECTING_ENDPOINT.get_or_init(|| Endpoint::uninit(EndpointID::External(External::Oem(92))));
        static ACCEPTING_ENDPOINT: OnceLock<Endpoint> = OnceLock::new();
        let accepting_endpoint =
            ACCEPTING_ENDPOINT.get_or_init(|| Endpoint::uninit(EndpointID::External(External::Oem(92))));

        register_endpoint(rejecting, rejecting_endpoint).await.unwrap();
        register_endpoint(accepting, accepting_endpoint).await.unwrap();

        // Snapshot the delegator-error counter so we can prove S2 wiring
        // bumped it on the rejection path.
        let errs_before = crate::metrics::comms::delegator_errors();

        // Send to the shared OEM(92) endpoint id.
        let payload: u32 = 0xDEADBEEF;
        let _ = send(
            EndpointID::External(External::Host),
            EndpointID::External(External::Oem(92)),
            &payload,
        )
        .await;

        // The rejecting delegate must have seen the call (proving the message
        // was actually dispatched, not just dropped at the router level).
        assert_eq!(
            rejecting.rejections.load(Ordering::SeqCst),
            1,
            "rejecting delegate must have been called"
        );
        // The accepting delegate must have received as well - proving the
        // route loop did not short-circuit on the rejecting delegate's Err.
        assert_eq!(
            accepting.hits.load(Ordering::SeqCst),
            1,
            "accepting delegate must still receive after another endpoint rejected"
        );

        // The delegator-error counter must have been bumped at least once
        // (the rejecting delegate returned BufferFull).
        let errs_after = crate::metrics::comms::delegator_errors();
        assert!(
            errs_after > errs_before,
            "comms::delegator_errors must increase after a rejecting delegate; before={} after={}",
            errs_before,
            errs_after,
        );
    }

    /// `route` must bump the `routed_unknown` counter when a message is sent
    /// to an `EndpointID` that has no registered receiver.
    #[tokio::test]
    async fn test_route_to_unknown_endpoint_bumps_counter() {
        init();

        // Pick an OEM key with no registered receiver. The internal lists
        // are shared across OEM keys, so we use a fresh key here; the
        // matching loop in `route` will check id-equality and fail to find
        // a receiver.
        let unknown = EndpointID::External(External::Oem(0xBEEF));

        let before = crate::metrics::comms::routed_unknown();

        let payload: u32 = 0xC0DE;
        let _ = send(EndpointID::External(External::Host), unknown, &payload).await;

        let after = crate::metrics::comms::routed_unknown();
        assert!(
            after > before,
            "comms::routed_unknown must increase after a send to an unregistered id; before={} after={}",
            before,
            after,
        );
    }

    /// `Endpoint::process` must bump the `delivered_to_uninit` counter and
    /// warn-log when a routed message reaches an endpoint whose delegator
    /// slot is still `None`. Under the single-executor model this is
    /// impossible via `register_endpoint` (push and init run with no yield
    /// between them), so we exercise the path by pushing an `Endpoint` to
    /// its list directly without ever calling `init`.
    #[tokio::test]
    async fn test_endpoint_with_uninitialized_delegator_bumps_counter() {
        init();

        // An Endpoint that has never had `init` called on it - delegator
        // slot remains None.
        static UNINIT_ENDPOINT: OnceLock<Endpoint> = OnceLock::new();
        let endpoint = UNINIT_ENDPOINT.get_or_init(|| Endpoint::uninit(EndpointID::External(External::Oem(93))));

        // Push directly to the list, bypassing register_endpoint so init is
        // never called.
        get_list(endpoint.id).get().await.push(endpoint).unwrap();

        let before = crate::metrics::comms::delivered_to_uninit();

        let payload: u32 = 0xFEED;
        let _ = send(
            EndpointID::External(External::Host),
            EndpointID::External(External::Oem(93)),
            &payload,
        )
        .await;

        let after = crate::metrics::comms::delivered_to_uninit();
        assert!(
            after > before,
            "comms::delivered_to_uninit must increase when a message reaches an uninitialized delegator; before={} after={}",
            before,
            after,
        );
    }
}
