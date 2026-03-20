use std::{future::Future, pin::pin};

use embassy_futures::select::select_slice;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, WaitResult},
};

use log::{info, warn};

/// Receive events from a [`Device`]
pub trait EventReceiver {
    fn wait_next(&mut self) -> impl Future<Output = Event>;
}

pub trait EventSender {
    fn on_plug(&self, power_mw: i32);
    fn on_unplug(&self);
}

/// Power device trait
pub trait Device {
    fn name(&self) -> &str;

    fn accept_contract(&mut self) -> impl Future<Output = ()>;
    fn disconnect(&mut self) -> impl Future<Output = ()>;
}

#[derive(Copy, Clone, Debug)]
pub struct NewContract {
    pub power_mw: i32,
}

#[derive(Copy, Clone, Debug)]
pub enum Event {
    Plug(NewContract),
    Unplug,
}

struct CurrentContract<'device, D: Device> {
    power_mw: i32,
    connected_device: &'device Mutex<NoopRawMutex, D>,
}

pub struct Sender<'channel> {
    publisher: DynImmediatePublisher<'channel, Event>,
}

impl<'channel> Sender<'channel> {
    pub fn new(publisher: DynImmediatePublisher<'channel, Event>) -> Self {
        Self { publisher }
    }
}

pub struct Receiver<'channel> {
    subscriber: DynSubscriber<'channel, Event>,
}

impl<'channel> Receiver<'channel> {
    pub fn new(subscriber: DynSubscriber<'channel, Event>) -> Self {
        Self { subscriber }
    }
}

impl EventReceiver for Receiver<'_> {
    async fn wait_next(&mut self) -> Event {
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
    fn on_plug(&self, power_mw: i32) {
        self.publisher.publish_immediate(Event::Plug(NewContract { power_mw }));
    }

    fn on_unplug(&self) {
        self.publisher.publish_immediate(Event::Unplug);
    }
}

pub struct ServiceImplementation<'storage, 'device, D: Device, R: EventReceiver> {
    current_connection: Option<CurrentContract<'device, D>>,
    devices: &'storage mut [(R, &'device Mutex<NoopRawMutex, D>)],
}

const MAX_SUPPORTED_DEVICES: usize = 4;

impl<'storage, 'device, D: Device, R: EventReceiver> ServiceImplementation<'storage, 'device, D, R> {
    pub fn new(devices: &'storage mut [(R, &'device Mutex<NoopRawMutex, D>)]) -> Self {
        Self {
            devices,
            current_connection: None,
        }
    }

    pub async fn wait_next(&mut self) -> (&'device Mutex<NoopRawMutex, D>, Event) {
        let futures =
            heapless::Vec::<_, MAX_SUPPORTED_DEVICES>::from_iter(self.devices.iter_mut().map(|(r, _)| r.wait_next()));

        let (event, index) = select_slice(pin!(futures)).await;
        (self.devices[index].1, event)
    }

    pub async fn process_event(&mut self, event: (&'device Mutex<NoopRawMutex, D>, Event)) {
        let mut event_device = event.0.lock().await;
        match event.1 {
            Event::Plug(data) => {
                info!("{} connected with contract: {:?}", event_device.name(), data.power_mw);
                if let Some(current) = &self.current_connection {
                    if data.power_mw > current.power_mw {
                        info!("New contract has higher power, switching");
                        current.connected_device.lock().await.disconnect().await;

                        self.current_connection = Some(CurrentContract {
                            power_mw: data.power_mw,
                            connected_device: event.0,
                        });
                        event_device.accept_contract().await;
                    } else {
                        info!("New contract has lower or equal power, not switching");
                    }
                } else {
                    info!("No current contract, accepting new one");
                    self.current_connection = Some(CurrentContract {
                        power_mw: data.power_mw,
                        connected_device: event.0,
                    });
                    event_device.accept_contract().await;
                }
            }
            Event::Unplug => {
                info!("{} disconnected", event_device.name());
                if let Some(current) = &self.current_connection {
                    if std::ptr::eq(current.connected_device, event.0) {
                        info!("Current device disconnected");
                        self.current_connection = None;
                    } else {
                        info!("A non-connected device unplugged, nothing to do");
                    }
                }
            }
        }
    }
}
