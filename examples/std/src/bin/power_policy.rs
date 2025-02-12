use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time as _;
use embedded_services::power::policy::{self, device, PowerCapability};
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
}

impl ExampleDevice {
    fn new(id: policy::DeviceId) -> Self {
        Self {
            device: policy::device::Device::new(id),
        }
    }

    async fn process_request(&self) -> Result<(), policy::Error> {
        match self.device.wait_request().await {
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
    let device0 = DEVICE0.get_or_init(|| ExampleDevice::new(policy::DeviceId(0)));
    policy::register_device(device0).await.unwrap();
    spawner.must_spawn(device_task0(device0));
    let device0 = device0.device.try_device_action().await.unwrap();

    info!("Creating device 1");
    static DEVICE1: OnceLock<ExampleDevice> = OnceLock::new();
    let device1 = DEVICE1.get_or_init(|| ExampleDevice::new(policy::DeviceId(1)));
    policy::register_device(device1).await.unwrap();
    spawner.must_spawn(device_task1(device1));
    let device1 = device1.device.try_device_action().await.unwrap();

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

    // Disable consumer device 0, device 1 should remain current consumer
    // Device 0 should not be able to consumer after device 1 is unplugged
    info!("Connecting device 0");
    device0.notify_consumer_power_capability(None).await.unwrap();
    let _device1 = device1.detach().await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task());
        spawner.must_spawn(run(spawner));
    });
}
