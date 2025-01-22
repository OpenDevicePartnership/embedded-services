//! Transport Service Definitions

use core::any::{Any, TypeId};
use core::cell::Cell;
use core::convert::Infallible;

use embassy_sync::once_lock::OnceLock;
use serde::{Deserialize, Serialize};

use crate::intrusive_list::{self, Node, NodeContainer};
use crate::IntrusiveList;

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

/// External identifier for transport routing
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

/// Endpoint identifier for transport routing
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Endpoint {
    /// route to/from an internal source
    Internal(Internal),

    /// route to/from an external source
    External(External),
}

impl From<Internal> for Endpoint {
    fn from(value: Internal) -> Self {
        Endpoint::Internal(value)
    }
}

impl From<External> for Endpoint {
    fn from(value: External) -> Self {
        Endpoint::External(value)
    }
}

/// Data reference -- generalized such that any stack variable can be transmitted "in place" as needed
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Data<'a> {
    contents: &'a dyn Any,
}

impl<'a> Data<'a> {
    /// Construct a Data portion of a Message from some data input
    pub fn new(from: &'a impl Any) -> Self {
        Self { contents: from }
    }

    /// Attempt to retrieve data as type T -- None if incorrect type
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.contents.downcast_ref()
    }

    /// Fetch type ID for message contents to allow reception of multiple top level elements
    /// Ex:
    /// match message.data.type_id() {
    ///     TypeId::of::<MessageClassA>() -> (),
    ///     TypeId::of::<MessageClassB>() -> (), etc.
    /// }
    pub fn type_id(&self) -> TypeId {
        self.contents.type_id()
    }

    /// Shorthand if only a few Message types are supported by an Endpoint:
    /// if data.is_a::<MessageClassA>() {}
    /// else if data.is_a::<MessageClassB>() {}
    /// etc.
    pub fn is_a<T: Any>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }
}

/// Message to receive
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Message<'a> {
    /// where this message came from
    pub from: Endpoint,

    /// where this message is going
    pub to: Endpoint,

    /// message content
    pub data: Data<'a>,
}

/// Receive trait for Registration implementers
pub trait MessageDelegate {
    /// Receive a Message (typically, push contents to queue or queue some action)
    fn process(&self, message: &Message);
}

/// Primary node registration for receiving messages from the transport service
pub struct EndpointLink {
    node: Node,
    who: Endpoint,
    delegator: Cell<Option<&'static dyn MessageDelegate>>,
}

impl NodeContainer for EndpointLink {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

impl EndpointLink {
    /// use this when static initialization occurs, internal fields will be validated in register_subscriber() later
    pub const fn uninit(who_am_i: Endpoint) -> Self {
        Self {
            node: Node::uninit(),
            who: who_am_i,
            delegator: Cell::new(None),
        }
    }

    /// Send a generic message to a Target
    pub async fn send(&self, to: Endpoint, data: &impl Any) -> Result<(), Infallible> {
        route(Message {
            from: self.who,
            to,
            data: Data::new(data),
        })
        .await
    }

    fn init(&self, rx: &'static dyn MessageDelegate) {
        self.delegator.set(Some(rx));
    }

    fn process(&self, message: &Message) {
        if let Some(delegator) = self.delegator.get() {
            delegator.process(message);
        }
    }
}

/// initialize receiver/transport node for message handling
pub async fn register_endpoint(
    this: &'static impl MessageDelegate,
    node: &'static EndpointLink,
) -> Result<(), intrusive_list::Error> {
    node.init(this);
    get_list(node.who).get().await.push(node)
}

fn get_list(target: Endpoint) -> &'static OnceLock<IntrusiveList> {
    match target {
        Endpoint::External(ext_endpoint) => match ext_endpoint {
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
        Endpoint::Internal(int_endpoint) => {
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

/// route a message to any valid receiver nodes
pub async fn route(message: Message<'_>) -> Result<(), Infallible> {
    let list = get_list(message.to).get().await;

    for rxq in list {
        if let Some(endpoint) = rxq.data::<EndpointLink>() {
            if message.to == endpoint.who {
                endpoint.process(&message);
            }
        }
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
