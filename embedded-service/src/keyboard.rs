//! Keyboard service data types and common functionality

use embassy_sync::mutex::Mutex;

use crate::GlobalRawMutex;
use crate::buffer::SharedRef;
use crate::comms::{self, EndpointID, External, Internal};

/// Keyboard device ID
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// Keyboard key
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Key(pub u8);

/// Key event data
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyEvent {
    /// Key release
    Break(Key),
    /// Key press
    Make(Key),
}

/// Keyboard event messages
#[derive(Clone)]
pub enum Event<'a> {
    /// Key press event
    KeyEvent(DeviceId, SharedRef<'a, KeyEvent>),
}

/// Top-level message data enum
#[derive(Clone)]
pub enum MessageData<'a> {
    /// Event
    Event(Event<'a>),
}

/// Top-level message struct
#[derive(Clone)]
pub struct Message<'a> {
    /// Target/source device ID
    pub device_id: DeviceId,
    /// Message data
    pub data: MessageData<'a>,
}

/// Broadcast target configuration
#[derive(Copy, Clone)]
pub struct BroadcastConfig {
    /// Enable broadcasting to the HID endpoint
    broadcast_hid: bool,
    /// Enable broadcasting to the host endpoint
    broadcast_host: bool,
}

impl BroadcastConfig {
    /// New default
    pub const fn new() -> Self {
        Self {
            broadcast_hid: false,
            broadcast_host: false,
        }
    }
}

impl Default for BroadcastConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Keyboard service context
struct Context {
    broadcast_config: Mutex<GlobalRawMutex, BroadcastConfig>,
}

static CONTEXT: Context = Context {
    broadcast_config: Mutex::new(BroadcastConfig::new()),
};

/// Initialize common keyboard service functionality
pub fn init() {}

/// Enable broadcasting messages to the host endpoint
pub async fn enable_broadcast_host() {
    let mut config = CONTEXT.broadcast_config.lock().await;
    config.broadcast_host = true;
}

/// Enable broadcasting messages to the HID endpoint
pub async fn enable_broadcast_hid() {
    let mut config = CONTEXT.broadcast_config.lock().await;
    config.broadcast_hid = true;
}

/// Broadcast a keyboard message to the specified endpoints
pub async fn broadcast_message_with_config(from: DeviceId, config: BroadcastConfig, data: MessageData<'static>) {
    let message = Message { device_id: from, data };

    if config.broadcast_hid {
        let _ = comms::send(
            EndpointID::Internal(Internal::Keyboard),
            EndpointID::Internal(Internal::Hid),
            &message,
        )
        .await;
    }

    if config.broadcast_host {
        let _ = comms::send(
            EndpointID::Internal(Internal::Keyboard),
            EndpointID::External(External::Host),
            &message,
        )
        .await;
    }
}

/// Broadcast a keyboard message using the global broadcast config
pub async fn broadcast_message(from: DeviceId, data: MessageData<'static>) {
    let config = *CONTEXT.broadcast_config.lock().await;
    broadcast_message_with_config(from, config, data).await;
}
