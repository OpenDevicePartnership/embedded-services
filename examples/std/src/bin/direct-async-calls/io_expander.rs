use std::{future::Future, pin::pin};

use embassy_futures::select::select_slice;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, WaitResult},
};
use log::{info, warn};

pub trait EventReceiver {
    fn wait_next(&mut self) -> impl Future<Output = InterruptEvent>;
}

pub trait EventSender {
    fn on_interrupt(&self, pin: u8, level: bool);
}

/// IO expander device trait
pub trait Device {
    fn name(&self) -> &str;

    fn set_level(&mut self, pin: u8, value: bool) -> impl Future<Output = ()>;
}

pub struct Sender<'channel> {
    publisher: DynImmediatePublisher<'channel, InterruptEvent>,
}

impl<'channel> Sender<'channel> {
    pub fn new(publisher: DynImmediatePublisher<'channel, InterruptEvent>) -> Self {
        Self { publisher }
    }
}

pub struct Receiver<'channel> {
    subscriber: DynSubscriber<'channel, InterruptEvent>,
}

impl<'channel> Receiver<'channel> {
    pub fn new(subscriber: DynSubscriber<'channel, InterruptEvent>) -> Self {
        Self { subscriber }
    }
}

impl EventReceiver for Receiver<'_> {
    async fn wait_next(&mut self) -> InterruptEvent {
        loop {
            match self.subscriber.next_message().await {
                WaitResult::Message(msg) => return msg,
                WaitResult::Lagged(n) => {
                    warn!("Receiver lagged by {n} messages");
                }
            }
        }
    }
}

impl EventSender for Sender<'_> {
    fn on_interrupt(&self, pin: u8, level: bool) {
        self.publisher.publish_immediate(InterruptEvent { pin, level });
    }
}

#[derive(Copy, Clone)]
pub struct InterruptEvent {
    pub pin: u8,
    pub level: bool,
}

const MAX_SUPPORTED_DEVICES: usize = 4;

pub struct ServiceImplementation<'storage, 'device, D: Device, R: EventReceiver> {
    devices: &'storage mut [(R, &'device Mutex<NoopRawMutex, D>)],
}

impl<'storage, 'device, D: Device, R: EventReceiver> ServiceImplementation<'storage, 'device, D, R> {
    pub fn new(devices: &'storage mut [(R, &'device Mutex<NoopRawMutex, D>)]) -> Self {
        Self { devices }
    }

    pub async fn wait_next(&mut self) -> (&'device Mutex<NoopRawMutex, D>, InterruptEvent) {
        let futures =
            heapless::Vec::<_, MAX_SUPPORTED_DEVICES>::from_iter(self.devices.iter_mut().map(|(r, _)| r.wait_next()));

        let (event, index) = select_slice(pin!(futures)).await;
        (self.devices[index].1, event)
    }

    pub async fn process_event(&mut self, event: (&'device Mutex<NoopRawMutex, D>, InterruptEvent)) {
        let mut device = event.0.lock().await;
        info!(
            "Interrupt from device {}: pin {}, level {}",
            device.name(),
            event.1.pin,
            event.1.level
        );
        if event.1.pin == 0 {
            info!("Asserting INT_OUT pin");
        }

        device.set_level(1, event.1.level).await;
    }
}
