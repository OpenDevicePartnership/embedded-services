use embassy_executor::Executor;
use embassy_futures::join::join3;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, pubsub::PubSubChannel};
use embassy_time::Timer;
use static_cell::StaticCell;

use crate::{io_expander::EventSender as _, power::EventSender as _};

mod io_expander;
mod power;

pub struct Device<I: io_expander::EventSender, P: power::EventSender> {
    name: &'static str,
    io_sender: I,
    power_sender: P,
}

pub struct DeviceContainer<I: io_expander::EventSender, P: power::EventSender> {
    inner: Mutex<NoopRawMutex, Device<I, P>>,
}

impl<I: io_expander::EventSender, P: power::EventSender> DeviceContainer<I, P> {
    pub fn new(name: &'static str, io_sender: I, power_sender: P) -> Self {
        Self {
            inner: Mutex::new(Device {
                name,
                io_sender,
                power_sender,
            }),
        }
    }
}

impl<I: io_expander::EventSender, P: power::EventSender> power::Device for Device<I, P> {
    fn name(&self) -> &str {
        self.name
    }

    async fn accept_contract(&mut self) {
        log::info!("{}: Contract accepted", self.name);
    }

    async fn disconnect(&mut self) {
        log::info!("{}: Device disconnected", self.name);
    }
}

impl<I: io_expander::EventSender, P: power::EventSender> io_expander::Device for Device<I, P> {
    fn name(&self) -> &str {
        self.name
    }

    async fn set_level(&mut self, pin: u8, value: bool) {
        log::info!("{}: Set pin {} to level {}", self.name, pin, value);
    }
}

#[embassy_executor::task]
async fn run() {
    let power_channel0: PubSubChannel<NoopRawMutex, power::Event, 4, 1, 1> = PubSubChannel::new();
    let power_receiver0 = power::Receiver::new(power_channel0.dyn_subscriber().unwrap());
    let power_sender0 = power::Sender::new(power_channel0.dyn_immediate_publisher());

    let io_channel0: PubSubChannel<NoopRawMutex, io_expander::InterruptEvent, 4, 1, 1> = PubSubChannel::new();
    let io_receiver0 = io_expander::Receiver::new(io_channel0.dyn_subscriber().unwrap());
    let io_sender0 = io_expander::Sender::new(io_channel0.dyn_immediate_publisher());
    let device0 = DeviceContainer::new("Device0", io_sender0, power_sender0);

    let power_channel1: PubSubChannel<NoopRawMutex, power::Event, 4, 1, 1> = PubSubChannel::new();
    let power_receiver1 = power::Receiver::new(power_channel1.dyn_subscriber().unwrap());
    let power_sender1 = power::Sender::new(power_channel1.dyn_immediate_publisher());

    let io_channel1: PubSubChannel<NoopRawMutex, io_expander::InterruptEvent, 4, 1, 1> = PubSubChannel::new();
    let io_receiver1 = io_expander::Receiver::new(io_channel1.dyn_subscriber().unwrap());
    let io_sender1 = io_expander::Sender::new(io_channel1.dyn_immediate_publisher());
    let device1 = DeviceContainer::new("Device1", io_sender1, power_sender1);

    let mut power_devices = [(power_receiver0, &device0.inner), (power_receiver1, &device1.inner)];
    let mut power_service = power::ServiceImplementation::new(&mut power_devices);

    let mut io_devices = [(io_receiver0, &device0.inner), (io_receiver1, &device1.inner)];
    let mut io_service = io_expander::ServiceImplementation::new(&mut io_devices);

    join3(
        async {
            loop {
                let event = io_service.wait_next().await;
                io_service.process_event(event).await;
            }
        },
        async {
            loop {
                let event = power_service.wait_next().await;
                power_service.process_event(event).await;
            }
        },
        async {
            device0.inner.lock().await.power_sender.on_plug(1000);
            device0.inner.lock().await.io_sender.on_interrupt(0, true);
            Timer::after_millis(500).await;

            device1.inner.lock().await.power_sender.on_plug(2000);
            Timer::after_millis(500).await;

            device0.inner.lock().await.power_sender.on_unplug();
            Timer::after_millis(500).await;

            device1.inner.lock().await.io_sender.on_interrupt(0, true);
            device1.inner.lock().await.power_sender.on_unplug();
            Timer::after_millis(500).await;
        },
    )
    .await;
}

pub fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run());
    });
}
