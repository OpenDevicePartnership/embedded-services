//! Keyboard service data types and common functionality

use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;

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
#[derive(Copy, Clone, Default)]
pub struct BroadcastConfig {
    /// Enable broadcasting to the HID endpoint
    broadcast_hid: bool,
    /// Enable broadcasting to the host endpoint
    broadcast_host: bool,
}

/// Keyboard service context
struct Context {
    broadcast_config: Mutex<GlobalRawMutex, BroadcastConfig>,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Initialize common keyboard service functionality
pub fn init() {
    CONTEXT.get_or_init(|| Context {
        broadcast_config: Mutex::new(BroadcastConfig::default()),
    });
}

/// Enable broadcasting messages to the host endpoint
pub async fn enable_broadcast_host() {
    let context = CONTEXT.get().await;

    let mut config = context.broadcast_config.lock().await;
    config.broadcast_host = true;
}

/// Enable broadcasting messages to the HID endpoint
pub async fn enable_broadcast_hid() {
    let context = CONTEXT.get().await;

    let mut config = context.broadcast_config.lock().await;
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
    let config = *CONTEXT.get().await.broadcast_config.lock().await;
    broadcast_message_with_config(from, config, data).await;
}
