use embassy_executor::{Executor, Spawner};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, once_lock::OnceLock, pubsub::PubSubChannel};
use embassy_time::{self as _, Timer};
use embedded_services::{
    broadcaster::immediate as broadcaster,
    power::policy::{self, ConsumerPowerCapability, PowerCapability, device, flags, policy::Context},
};
use log::*;
use static_cell::StaticCell;

const LOW_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

const HIGH_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

const POWER_POLICY_CHANNEL_SIZE: usize = 1;
const NUM_POWER_DEVICES: usize = 2;

struct ExampleDevice {
    device: policy::device::Device<POWER_POLICY_CHANNEL_SIZE>,
}

impl ExampleDevice {
    fn new(id: policy::DeviceId, context_ref: &'static Context<POWER_POLICY_CHANNEL_SIZE>) -> Self {
        Self {
            device: policy::device::Device::new(id, context_ref),
        }
    }

    async fn process_request(&self) -> Result<(), policy::Error> {
        let request = self.device.receive().await;
        match request.command {
            device::CommandData::ConnectAsConsumer(capability) => {
                info!(
                    "Device {} received connect consumer at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::CommandData::ConnectAsProvider(capability) => {
                info!(
                    "Device {} received connect provider at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::CommandData::Disconnect => {
                info!("Device {} received disconnect", self.device.id().0);
            }
        }

        request.respond(Ok(policy::device::ResponseData::Complete));
        Ok(())
    }
}

impl policy::device::DeviceContainer<POWER_POLICY_CHANNEL_SIZE> for ExampleDevice {
    fn get_power_policy_device(&self) -> &policy::device::Device<POWER_POLICY_CHANNEL_SIZE> {
        &self.device
    }
}

#[embassy_executor::task]
async fn device_task0(device: &'static ExampleDevice) {
    loop {
        if let Err(e) = device.process_request().await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn device_task1(device: &'static ExampleDevice) {
    loop {
        if let Err(e) = device.process_request().await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner, service: &'static power_policy_service::PowerPolicy<POWER_POLICY_CHANNEL_SIZE>) {
    embedded_services::init().await;

    spawner.must_spawn(receiver_task(service));

    info!("Creating device 0");
    static DEVICE0: OnceLock<ExampleDevice> = OnceLock::new();
    let device0_mock = DEVICE0.get_or_init(|| ExampleDevice::new(policy::DeviceId(0), &service.context));
    spawner.must_spawn(device_task0(device0_mock));
    let device0 = device0_mock.device.try_device_action().await.unwrap();

    info!("Creating device 1");
    static DEVICE1: OnceLock<ExampleDevice> = OnceLock::new();
    let device1_mock = DEVICE1.get_or_init(|| ExampleDevice::new(policy::DeviceId(1), &service.context));
    spawner.must_spawn(device_task1(device1_mock));
    let device1 = device1_mock.device.try_device_action().await.unwrap();

    spawner.must_spawn(power_policy_service_task(service, [device0_mock, device1_mock]));

    // Plug in device 0, should become current consumer
    info!("Connecting device 0");
    let device0 = device0.attach().await.unwrap();
    device0
        .notify_consumer_power_capability(Some(ConsumerPowerCapability {
            capability: LOW_POWER,
            flags: flags::Consumer::none().with_unconstrained_power(),
        }))
        .await
        .unwrap();

    // Plug in device 1, should become current consumer
    info!("Connecting device 1");
    let device1 = device1.attach().await.unwrap();
    device1
        .notify_consumer_power_capability(Some(HIGH_POWER.into()))
        .await
        .unwrap();

    // Unplug device 0, device 1 should remain current consumer
    info!("Unpluging device 0");
    let device0 = device0.detach().await.unwrap();

    // Plug in device 0, device 1 should remain current consumer
    info!("Connecting device 0");
    let device0 = device0.attach().await.unwrap();
    device0
        .notify_consumer_power_capability(Some(LOW_POWER.into()))
        .await
        .unwrap();

    // Unplug device 1, device 0 should become current consumer
    info!("Unplugging device 1");
    let device1 = device1.detach().await.unwrap();

    // Replug device 1, device 1 becomes current consumer
    info!("Connecting device 1");
    let device1 = device1.attach().await.unwrap();
    device1
        .notify_consumer_power_capability(Some(HIGH_POWER.into()))
        .await
        .unwrap();

    // Disconnect consumer device 0, device 1 should remain current consumer
    // Device 0 should not be able to consume after device 1 is unplugged
    info!("Disconnecting device 0");
    device0.notify_consumer_power_capability(None).await.unwrap();
    let device1 = device1.detach().await.unwrap();

    // Switch to provider on device0
    info!("Device 0 requesting provider");
    device0
        .request_provider_power_capability(LOW_POWER.into())
        .await
        .unwrap();
    Timer::after_millis(250).await;
    info!(
        "Total provider power: {} mW",
        service.context.compute_total_provider_power_mw().await
    );

    info!("Device 1 attach and requesting provider");
    let device1 = device1.attach().await.unwrap();
    device1
        .request_provider_power_capability(LOW_POWER.into())
        .await
        .unwrap();
    // Wait for the provider to be connected
    Timer::after_millis(250).await;
    info!(
        "Total provider power: {} mW",
        service.context.compute_total_provider_power_mw().await
    );

    // Provider upgrade should fail because device 0 is already connected
    info!("Device 1 attempting provider upgrade");
    device1
        .request_provider_power_capability(HIGH_POWER.into())
        .await
        .unwrap();
    // Wait for the upgrade flow to complete
    Timer::after_millis(250).await;
    info!(
        "Total provider power: {} mW",
        service.context.compute_total_provider_power_mw().await
    );

    // Disconnect device 0
    info!("Device 0 disconnecting");
    device0.detach().await.unwrap();
    // Wait for the detach flow to complete
    Timer::after_millis(250).await;
    info!(
        "Total provider power: {} mW",
        service.context.compute_total_provider_power_mw().await
    );

    // Provider upgrade should succeed now
    info!("Device 1 attempting provider upgrade");
    device1
        .request_provider_power_capability(HIGH_POWER.into())
        .await
        .unwrap();
    // Wait for the upgrade flow to complete
    Timer::after_millis(250).await;
    info!(
        "Total provider power: {} mW",
        service.context.compute_total_provider_power_mw().await
    );
}

#[embassy_executor::task]
async fn receiver_task(service: &'static power_policy_service::PowerPolicy<POWER_POLICY_CHANNEL_SIZE>) {
    static CHANNEL: StaticCell<PubSubChannel<NoopRawMutex, policy::CommsMessage, 4, 1, 0>> = StaticCell::new();
    let channel = CHANNEL.init(PubSubChannel::new());

    let publisher = channel.dyn_immediate_publisher();
    let mut subscriber = channel.dyn_subscriber().unwrap();

    static RECEIVER: StaticCell<broadcaster::Receiver<'static, policy::CommsMessage>> = StaticCell::new();
    let receiver = RECEIVER.init(broadcaster::Receiver::new(publisher));

    service.context.register_message_receiver(receiver).unwrap();

    loop {
        match subscriber.next_message().await {
            embassy_sync::pubsub::WaitResult::Message(msg) => {
                info!("Received message: {msg:?}");
            }
            embassy_sync::pubsub::WaitResult::Lagged(count) => {
                warn!("Lagged messages: {count}");
            }
        }
    }
}

#[embassy_executor::task]
async fn power_policy_service_task(
    service: &'static power_policy_service::PowerPolicy<POWER_POLICY_CHANNEL_SIZE>,
    devices: [&'static ExampleDevice; NUM_POWER_DEVICES],
) {
    power_policy_service::task::task(service, Some(devices), None::<[&std_examples::type_c::DummyCharger; 0]>)
        .await
        .expect("Failed to start power policy service task");
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    static SERVICE: StaticCell<power_policy_service::PowerPolicy<POWER_POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let service = SERVICE.init(power_policy_service::PowerPolicy::new(
        power_policy_service::config::Config::default(),
    ));

    executor.run(|spawner| {
        spawner.must_spawn(run(spawner, service));
    });
}
