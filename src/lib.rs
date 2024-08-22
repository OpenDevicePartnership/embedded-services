//! Embedded Services Interface Exports

#![no_std]
#![warn(missing_docs)]

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pubsub::{DynPublisher, DynSubscriber, PubSubChannel};

pub struct Publisher<'a, T: Clone>(DynPublisher<'a, T>);
pub struct Subscriber<'a, T: Clone>(DynSubscriber<'a, T>);

pub trait Service {
    type Notification: Clone;
    type Config;

    fn init(config: Self::Config) -> Self;
}

pub trait DynamicServiceInterface<T: Service> {
    fn subscribe(&self) -> Result<Subscriber<'_, T::Notification>, OutOfSubscriptionSlots>;
    fn register_publisher(&self) -> Result<Publisher<'_, T::Notification>, OutOfPublisherSlots>;
}

pub struct DynamicService<T: Service, const SUBS: usize, const PUBS: usize> {
    inner: T,
    chn: PubSubChannel<NoopRawMutex, T::Notification, 1, SUBS, PUBS>,
}

pub fn configure<T: Service, const SUBS: usize, const PUBS: usize>(config: T::Config) -> DynamicService<T, SUBS, PUBS> {
    DynamicService {
        inner: T::init(config),
        chn: PubSubChannel::new(),
    }
}

#[derive(Copy, Clone, Debug)]
pub struct OutOfSubscriptionSlots();
#[derive(Copy, Clone, Debug)]
pub struct OutOfPublisherSlots();

impl<T: Service, const SUBS: usize, const PUBS: usize> DynamicServiceInterface<T> for DynamicService<T, SUBS, PUBS> {
    fn subscribe(&self) -> Result<Subscriber<'_, T::Notification>, OutOfSubscriptionSlots> {
        match self.chn.dyn_subscriber() {
            Ok(sub) => Ok(Subscriber(sub)),
            Err(_) => Err(OutOfSubscriptionSlots()),
        }
    }

    fn register_publisher(&self) -> Result<Publisher<'_, T::Notification>, OutOfPublisherSlots> {
        match self.chn.dyn_publisher() {
            Ok(pbl) => Ok(Publisher(pbl)),
            Err(_) => Err(OutOfPublisherSlots()),
        }
    }
}

impl<'a, T: Clone> Subscriber<'a, T> {
    pub async fn wait(&mut self) -> T {
        self.0.next_message_pure().await
    }
}

impl<'a, T: Clone> Publisher<'a, T> {
    pub async fn publish(&self, notification: T) {
        self.0.publish(notification).await;
    }
}

pub mod activity;
pub enum DynamicServiceListing {
    Activity,
    OEM(usize),
}

pub enum DynamicServiceInstance<'a> {
    Activity(&'a dyn DynamicServiceInterface<activity::Manager>),
}

pub struct ReadRequest {}
pub trait DynamicServiceBlock {
    fn get(&self, service: DynamicServiceListing) -> Option<DynamicServiceInstance<'_>>;
}

pub struct Services<T: DynamicServiceBlock> {
    pub read: ReadRequest,
    pub dynamic: T,
}

pub fn init<T: DynamicServiceBlock>(dynamic_allocated: T) -> Services<T> {
    Services {
        read: ReadRequest::new(),
        dynamic: dynamic_allocated,
    }
}

impl ReadRequest {
    fn new() -> ReadRequest {
        let mut r = ReadRequest {};
        // r._name_::init(&mut r._name_);
        r
    }
}
