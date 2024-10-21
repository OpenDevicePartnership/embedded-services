use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::{
    hid::{self, DeviceId},
    transport::{self, Endpoint, Internal},
};
use log::*;
use static_cell::StaticCell;

const DEV0_ID: DeviceId = DeviceId(0);
const DEV1_ID: DeviceId = DeviceId(1);

struct Host {
    tp: transport::EndpointLink,
}

impl Host {
    fn new() -> Self {
        Host {
            tp: transport::EndpointLink::uninit(Endpoint::Internal(Internal::Hid)),
        }
    }
}

impl transport::MessageDelegate for Host {
    fn process(&self, _message: &transport::Message) {}
}

struct Device {
    tp: transport::EndpointLink,
    id: DeviceId,
}

impl Device {
    fn new(id: DeviceId) -> Self {
        Device {
            tp: transport::EndpointLink::uninit(Endpoint::Internal(Internal::Hid)),
            id,
        }
    }
}

impl transport::MessageDelegate for Device {
    fn process(&self, message: &transport::Message) {
        let message = message.data.get::<hid::Message>();
        if message.is_none() {
            return;
        }

        if message.unwrap().id != self.id {
            return;
        }

        info!("{:?} got message", self.id);
    }
}

#[embassy_executor::task]
async fn host() {
    static HOST: OnceLock<Host> = OnceLock::new();
    let this = HOST.get_or_init(|| Host::new());

    info!("Registering host endpoint");
    transport::register_endpoint(this, &this.tp).await.unwrap();

    loop {
        info!("Sending message");
        hid::send_request(&this.tp, DEV0_ID, hid::Request::HidDescriptor).await;
        hid::send_request(&this.tp, DEV1_ID, hid::Request::HidDescriptor).await;
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    static DEVICE0: OnceLock<Device> = OnceLock::new();
    static DEVICE1: OnceLock<Device> = OnceLock::new();

    embedded_services::init().await;

    info!("Registering device 0 endpoint");
    let dev0 = DEVICE0.get_or_init(|| Device::new(DEV0_ID));
    transport::register_endpoint(dev0, &dev0.tp).await.unwrap();

    info!("Registering device 1 endpoint");
    let dev1 = DEVICE1.get_or_init(|| Device::new(DEV1_ID));
    transport::register_endpoint(dev1, &dev1.tp).await.unwrap();

    info!("Spawning host task");
    spawner.spawn(host()).unwrap();

    loop {
        embassy_time::Timer::after_secs(10).await;
        info!("10s elapsed...");
    }
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(run(spawner)).unwrap();
    });
}
