use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::{
    comms::{self, EndpointID, Internal},
    hid::{self, DeviceId},
};
use log::*;
use static_cell::StaticCell;
const DEV0_ID: DeviceId = DeviceId(0);
const DEV1_ID: DeviceId = DeviceId(1);
struct Host {
    tp: comms::Endpoint,
}
impl Host {
    fn new() -> Self {
        Host {
            tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Hid)),
        }
    }
}

impl comms::MailboxDelegate for Host {}

struct Device {
    tp: comms::Endpoint,
    id: DeviceId,
}
impl Device {
    fn new(id: DeviceId) -> Self {
        Device {
            tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Hid)),
            id,
        }
    }
}
impl comms::MailboxDelegate for Device {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        let message = message
            .data
            .get::<hid::Message>()
            .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

        if message.id != self.id {
            Err(comms::MailboxDelegateError::InvalidId)
        } else {
            info!("{:?} got message", self.id);
            Ok(())
        }
    }
}
#[embassy_executor::task]
async fn host() {
    static HOST: OnceLock<Host> = OnceLock::new();
    let this = HOST.get_or_init(Host::new);
    info!("Registering host endpoint");
    comms::register_endpoint(this, &this.tp).await.unwrap();
    loop {
        info!("Sending message");
        hid::send_request(&this.tp, DEV0_ID, hid::Request::Descriptor)
            .await
            .unwrap();
        hid::send_request(&this.tp, DEV1_ID, hid::Request::Descriptor)
            .await
            .unwrap();
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
    comms::register_endpoint(dev0, &dev0.tp).await.unwrap();
    info!("Registering device 1 endpoint");
    let dev1 = DEVICE1.get_or_init(|| Device::new(DEV1_ID));
    comms::register_endpoint(dev1, &dev1.tp).await.unwrap();
    info!("Spawning host task");
    spawner.spawn(host()).unwrap();
}
static EXECUTOR: StaticCell<Executor> = StaticCell::new();
fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(run(spawner)).unwrap();
    });
}
