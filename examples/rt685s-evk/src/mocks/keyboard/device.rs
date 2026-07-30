//! Mock HID keyboard device used by the `mock_i2c_keyboard` example.
//!
//! Unlike the mouse mock, the keyboard copies each report out of a regular channel, which is simpler,
//! but can be costly for larger reports.

use super::interface::*;
use embedded_services::{info, warn};

/// Depth of the keyboard input-report channel. In a production use case, you may want to make the service generic over this value rather than hardcoding it.
const KEYBOARD_CHANNEL_DEPTH: usize = 5;

struct MockKeyboardServiceResourcesInner<const MAX_SUBS: usize> {
    channel: embassy_sync::pubsub::PubSubChannel<
        embedded_services::GlobalRawMutex,
        KeyboardInputReport,
        KEYBOARD_CHANNEL_DEPTH,
        MAX_SUBS,
        1,
    >,
}

pub struct MockKeyboardServiceResources<const MAX_SUBS: usize> {
    inner: Option<MockKeyboardServiceResourcesInner<MAX_SUBS>>,
}

impl<const MAX_SUBS: usize> Default for MockKeyboardServiceResources<MAX_SUBS> {
    fn default() -> Self {
        Self {
            inner: Some(MockKeyboardServiceResourcesInner {
                channel: embassy_sync::pubsub::PubSubChannel::new(),
            }),
        }
    }
}

/// Consumer side of the mock keyboard. Owns the channel that carries input reports and exposes a
/// method to inject key presses.
pub struct MockKeyboardService<'hw, const MAX_SUBS: usize> {
    resources: &'hw MockKeyboardServiceResourcesInner<MAX_SUBS>,
}

impl<'hw, const MAX_SUBS: usize> MockKeyboardService<'hw, MAX_SUBS> {
    pub fn new(
        resources: &'hw mut MockKeyboardServiceResources<MAX_SUBS>,
    ) -> (Self, MockKeyboardServiceRunner<'hw, MAX_SUBS>) {
        let resources = resources.inner.insert(MockKeyboardServiceResourcesInner {
            channel: embassy_sync::pubsub::PubSubChannel::new(),
        });
        (
            Self { resources },
            MockKeyboardServiceRunner {
                publisher: resources
                    .channel
                    .publisher()
                    .expect("We know there's a free publisher because we just created the channel"),
            },
        )
    }
}

pub struct MockKeyboardServiceRunner<'hw, const MAX_SUBS: usize> {
    publisher: embassy_sync::pubsub::Publisher<
        'hw,
        embedded_services::GlobalRawMutex,
        KeyboardInputReport,
        KEYBOARD_CHANNEL_DEPTH,
        MAX_SUBS,
        1,
    >,
}

// Note: on a normal keyboard service, this would just have a run() method that actually does the keyscanning.
// For the mock, we just let the user inject keystrokes instead.
impl<'hw, const MAX_SUBS: usize> MockKeyboardServiceRunner<'hw, MAX_SUBS> {
    pub async fn click_key(&self, key_code: KeyCode) {
        // key down
        let send_result = self.publisher.try_publish(KeyboardInputReport {
            modifiers: 0,
            reserved: 0,
            keys: [key_code.into(), 0, 0, 0, 0, 0],
        });

        if let Err(e) = send_result {
            warn!("Failed to send key down report: {:?}", e);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_millis(15)).await;

        // key up
        let send_result = self.publisher.try_publish(KeyboardInputReport::default());
        if let Err(e) = send_result {
            warn!("Failed to send key up report: {:?}", e);
        }
    }
}

impl<'hw, const MAX_SUBS: usize> KeyboardService<'hw> for MockKeyboardService<'hw, MAX_SUBS> {
    async fn set_led(&mut self, state: u8) {
        info!("Setting LED state (mock) - no-op.  New state:");
        info!(
            "NumLock: {}, CapsLock: {}, ScrollLock: {}",
            state & (KeyboardLedFlags::NumLock as u8) != 0,
            state & (KeyboardLedFlags::CapsLock as u8) != 0,
            state & (KeyboardLedFlags::ScrollLock as u8) != 0
        );
    }

    fn subscriber(
        &self,
    ) -> Result<embassy_sync::pubsub::DynSubscriber<'hw, KeyboardInputReport>, embassy_sync::pubsub::Error> {
        self.resources.channel.dyn_subscriber()
    }
}
