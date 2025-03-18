use std::cell::Cell;

use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::{self as _, Timer};
use embedded_services::power::policy::{self, action::ConnectedProvider, device, PowerCapability};
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

struct ExampleDevice {
    device: policy::device::Device,
    /// Flag to reject the next n provider request
    reject_requests: Cell<i32>,
}

impl ExampleDevice {
    fn new(id: policy::DeviceId) -> Self {
        Self {
            device: policy::device::Device::new(id),
            reject_requests: Cell::new(0),
        }
    }

    fn reject_next_requests(&self, n: i32) {
        self.reject_requests.set(n);
    }

    async fn process_request(&self) -> Result<(), policy::Error> {
        let request = self.device.wait_request().await;
        if self.reject_requests.get() > 0 {
            info!("Rejecting request");
            self.reject_requests.set(self.reject_requests.get() - 1);
            self.device.send_response(Err(policy::Error::Failed)).await;
            return Ok(());
        }

        match request {
            device::RequestData::ConnectConsumer(capability) => {
                info!(
                    "Device {} received connect consumer at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::RequestData::ConnectProvider(capability) => {
                info!(
                    "Device {} received connect source at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::RequestData::Disconnect => {
                info!("Device {} received disconnect", self.device.id().0);
            }
        }

        self.device.send_response(Ok(device::ResponseData::Complete)).await;
        Ok(())
    }
}

impl policy::device::DeviceContainer for ExampleDevice {
    fn get_power_policy_device(&self) -> &policy::device::Device {
        &self.device
    }
}

#[embassy_executor::task]
async fn device_task0(device: &'static ExampleDevice) {
    loop {
        if let Err(e) = device.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn device_task1(device: &'static ExampleDevice) {
    loop {
        if let Err(e) = device.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0: OnceLock<ExampleDevice> = OnceLock::new();
    let device0_mock = DEVICE0.get_or_init(|| ExampleDevice::new(policy::DeviceId(0)));
    policy::register_device(device0_mock).await.unwrap();
    spawner.must_spawn(device_task0(device0_mock));
    let device0 = device0_mock.device.try_device_action().await.unwrap();

    info!("Creating device 1");
    static DEVICE1: OnceLock<ExampleDevice> = OnceLock::new();
    let device1_mock = DEVICE1.get_or_init(|| ExampleDevice::new(policy::DeviceId(1)));
    policy::register_device(device1_mock).await.unwrap();
    spawner.must_spawn(device_task1(device1_mock));
    let device1 = device1_mock.device.try_device_action().await.unwrap();

    // Plug in device 0, should become current consumer
    info!("Connecting device 0");
    let device0 = device0.attach().await.unwrap();
    device0.notify_consumer_power_capability(Some(LOW_POWER)).await.unwrap();

    // Plug in device 1, should become current consumer
    info!("Connecting device 1");
    let device1 = device1.attach().await.unwrap();
    device1
        .notify_consumer_power_capability(Some(HIGH_POWER))
        .await
        .unwrap();

    // Unplug device 0, device 1 should remain current consumer
    info!("Unpluging device 0");
    let device0 = device0.detach().await.unwrap();

    // Plug in device 0, device 1 should remain current consumer
    info!("Connecting device 0");
    let device0 = device0.attach().await.unwrap();
    device0.notify_consumer_power_capability(Some(LOW_POWER)).await.unwrap();

    // Unplug device 1, device 0 should become current consumer
    info!("Unplugging device 1");
    let device1 = device1.detach().await.unwrap();

    // Replug device 1, device 1 becomes current consumer
    info!("Connecting device 1");
    let device1 = device1.attach().await.unwrap();
    device1
        .notify_consumer_power_capability(Some(HIGH_POWER))
        .await
        .unwrap();

    // Disconnect consumer device 0, device 1 should remain current consumer
    // Device 0 should not be able to consume after device 1 is unplugged
    info!("Connecting device 0");
    device0.notify_consumer_power_capability(None).await.unwrap();
    let device1 = device1.detach().await.unwrap();

    // Switch to provider on device0
    info!("Device 0 requesting provider");
    device0.request_provider_power_capability(LOW_POWER).await.unwrap();
    Timer::after_millis(250).await;

    info!("Device 1 attach and requesting provider");
    let device1 = device1.attach().await.unwrap();
    device1.request_provider_power_capability(LOW_POWER).await.unwrap();

    Timer::after_millis(250).await;
    let device0 = device0.detach().await.unwrap();

    Timer::after_millis(250).await;
    let device1 = device1.detach().await.unwrap();

    // Go through provider recovery flow
    info!("Recovery Flow");
    let device0 = device0.attach().await.unwrap();
    device0.request_provider_power_capability(LOW_POWER).await.unwrap();
    Timer::after_millis(250).await;
    // Requests we're rejecting:
    // Connect request
    // Disconnect request after failing connect request
    // Request to connect at recovery limit
    // Disconnect request after failing recovery limit
    // First recovery disconnect from `attempt_provider_recovery`
    // Next recovery disconnect completes
    info!("Rejecting next 5 requests");
    device0_mock.reject_next_requests(5);

    // Attach device 1, device 0 will fail provider connect request and trigger recovery flow
    let device1 = device1.attach().await.unwrap();
    device1.request_provider_power_capability(LOW_POWER).await.unwrap();

    // Wait for the recovery flow to start
    while !device0_mock.device.is_in_recovery().await {
        info!("Waiting for recovery flow to start");
        Timer::after_millis(100).await;
    }

    // Wait for the recovery flow to complete
    while device0_mock.device.is_in_recovery().await {
        info!("Waiting for recovery flow to complete");
        Timer::after_millis(100).await;
    }

    // Reconnect device 0
    info!("Reconnecting device 0 as provider");
    device0.request_provider_power_capability(LOW_POWER).await.unwrap();
    // Wait for device 0 to reconnect
    while !device0_mock.device.is_provider().await {
        info!("Waiting for device 0 to reconnect");
        Timer::after_millis(1000).await;
    }

    // Disconnect device 1
    info!("Disconnecting device 1");
    let device1 = device1_mock
        .device
        .try_device_action::<ConnectedProvider>()
        .await
        .unwrap();
    device1.disconnect().await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task(
            power_policy_service::config::Config::default(),
        ));
        spawner.must_spawn(run(spawner));
    });
}
