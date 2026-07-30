//! Mock HID mouse device used by the `mock_i2c_mouse` example.

use super::interface::*;
use embedded_services::warn;

/// Depth of the mouse input-report channel. In a production use case, you may want to make the service generic over this value rather than hardcoding it.
const MOUSE_CHANNEL_DEPTH: usize = 4;

const MOUSE_BUTTON_1: u8 = 0x01;
#[allow(dead_code)]
const MOUSE_BUTTON_2: u8 = 0x02;
#[allow(dead_code)]
const MOUSE_BUTTON_3: u8 = 0x04;

struct MockMouseServiceResourcesInner<const MAX_SUBS: usize> {
    channel: embassy_sync::pubsub::PubSubChannel<
        embedded_services::GlobalRawMutex,
        MouseInputReport,
        MOUSE_CHANNEL_DEPTH,
        MAX_SUBS,
        1,
    >,
}

pub struct MockMouseServiceResources<const MAX_SUBS: usize> {
    inner: Option<MockMouseServiceResourcesInner<MAX_SUBS>>,
}

impl<const MAX_SUBS: usize> Default for MockMouseServiceResources<MAX_SUBS> {
    fn default() -> Self {
        Self {
            inner: Some(MockMouseServiceResourcesInner {
                channel: embassy_sync::pubsub::PubSubChannel::new(),
            }),
        }
    }
}

/// Consumer side of the mock mouse. Owns the channel that carries input reports and exposes methods
/// to inject mouse events.
pub struct MockMouseService<'hw, const MAX_SUBS: usize> {
    resources: &'hw MockMouseServiceResourcesInner<MAX_SUBS>,
}

impl<'hw, const MAX_SUBS: usize> MockMouseService<'hw, MAX_SUBS> {
    pub fn new(
        resources: &'hw mut MockMouseServiceResources<MAX_SUBS>,
    ) -> (Self, MockMouseServiceRunner<'hw, MAX_SUBS>) {
        let resources = resources.inner.insert(MockMouseServiceResourcesInner {
            channel: embassy_sync::pubsub::PubSubChannel::new(),
        });
        (
            Self { resources },
            MockMouseServiceRunner {
                publisher: resources
                    .channel
                    .publisher()
                    .expect("We know there's a free publisher because we just created the channel"),
            },
        )
    }
}

pub struct MockMouseServiceRunner<'hw, const MAX_SUBS: usize> {
    publisher: embassy_sync::pubsub::Publisher<
        'hw,
        embedded_services::GlobalRawMutex,
        MouseInputReport,
        MOUSE_CHANNEL_DEPTH,
        MAX_SUBS,
        1,
    >,
}

// Note: on a normal mouse service, this would just have a run() method that actually polls the sensor.
// For the mock, we just let the user inject mouse events instead.
impl<'hw, const MAX_SUBS: usize> MockMouseServiceRunner<'hw, MAX_SUBS> {
    async fn send(&self, report: MouseInputReport) {
        if let Err(e) = self.publisher.try_publish(report) {
            warn!("Failed to send mouse report: {:?}", e);
        }
    }

    pub async fn send_click(&self) {
        // Mouse down
        self.send(MouseInputReport {
            buttons: MOUSE_BUTTON_1,
            x: 0,
            y: 0,
        })
        .await;

        embassy_time::Timer::after(embassy_time::Duration::from_millis(15)).await;

        // Mouse up
        self.send(MouseInputReport { buttons: 0, x: 0, y: 0 }).await;
    }

    pub async fn move_mouse(&self) {
        self.send(MouseInputReport {
            buttons: MOUSE_BUTTON_1,
            x: 10,
            y: 10,
        })
        .await;
    }
}

impl<'hw, const MAX_SUBS: usize> MouseService<'hw> for MockMouseService<'hw, MAX_SUBS> {
    fn subscriber(
        &self,
    ) -> Result<embassy_sync::pubsub::DynSubscriber<'hw, MouseInputReport>, embassy_sync::pubsub::Error> {
        self.resources.channel.dyn_subscriber()
    }
}
