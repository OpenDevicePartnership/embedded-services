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

    /// Time alarm service provider
    TimeAlarm,

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
    pub fn is_a<T: Any>(&self) -> bool {
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
    delegator: SyncCell<Option<&'static dyn MailboxDelegate>>,
}

impl NodeContainer for Endpoint {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

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
    pub async fn send(&self, to: EndpointID, data: &impl Any) -> Result<(), Infallible> {
        send(self.id, to, data).await
    }

    fn init(&self, rx: &'static dyn MailboxDelegate) {
        self.delegator.set(Some(rx));
    }

    fn process(&self, message: &Message) {
        if let Some(delegator) = self.delegator.get() {
            // REVISIT: Continue to propagate error
            let _res = delegator.receive(message);
        }
    }
}

/// initialize receiver node for message handling
pub async fn register_endpoint(
    this: &'static impl MailboxDelegate,
    node: &'static Endpoint,
) -> Result<(), intrusive_list::Error> {
    node.init(this);
    get_list(node.id).get().await.push(node)
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
            static INTERNAL_LIST_TIME_ALARM: OnceLock<IntrusiveList> = OnceLock::new();
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
                TimeAlarm => &INTERNAL_LIST_TIME_ALARM,
                Oem(_key) => &INTERNAL_LIST_OEM,
            }
        }
    }
}

/// Send a generic message to an endpoint
pub async fn send(from: EndpointID, to: EndpointID, data: &impl Any) -> Result<(), Infallible> {
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

    for rxq in list {
        if let Some(endpoint) = rxq.data::<Endpoint>() {
            if message.to == endpoint.id {
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
    get_list(Internal::TimeAlarm.into()).get_or_init(IntrusiveList::new);
    get_list(Internal::Oem(0).into()).get_or_init(IntrusiveList::new);

    // initialize external subscriber lists
    get_list(External::Debug.into()).get_or_init(IntrusiveList::new);
    get_list(External::Host.into()).get_or_init(IntrusiveList::new);
    get_list(External::Oem(0).into()).get_or_init(IntrusiveList::new);
}
