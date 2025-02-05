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
            device::RequestData::ConnectSink(capability) => {
                info!(
                    "Device {} received connect sink at {:#?}",
                    self.device.id().0,
                    capability
                );
            }
            device::RequestData::ConnectSource(capability) => {
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

    async fn plug(&self, capability: Option<PowerCapability>) -> Result<(), policy::Error> {
        self.device.policy().notify_attached().await?;
        self.device.policy().notify_sink_power_capability(capability).await
    }

    async fn unplug(&self) -> Result<(), policy::Error> {
        self.device.policy().notify_detached().await
    }

    async fn disable_sink(&self) -> Result<(), policy::Error> {
        self.device.policy().notify_sink_power_capability(None).await
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

    info!("Creating device 1");
    static DEVICE1: OnceLock<ExampleDevice> = OnceLock::new();
    let device1 = DEVICE1.get_or_init(|| ExampleDevice::new(policy::DeviceId(1)));
    policy::register_device(device1).await.unwrap();
    spawner.must_spawn(device_task1(device1));

    // Plug in device 0, should become current sink
    info!("Connecting device 0");
    device0.plug(Some(LOW_POWER)).await.unwrap();

    // Plug in device 1, should become current sink
    info!("Connecting device 1");
    device1.plug(Some(HIGH_POWER)).await.unwrap();

    // Unplug device 0, device 1 should remain current sink
    info!("Disconnecting device 0");
    device0.unplug().await.unwrap();

    // Plug in device 0, device 1 should remain current sink
    info!("Connecting device 0");
    device0.plug(Some(LOW_POWER)).await.unwrap();

    // Unplug device 1, device 0 should become current sink
    info!("Disconnecting device 1");
    device1.unplug().await.unwrap();

    // Replug device 1, device 1 becomes current sink
    info!("Connecting device 1");
    device1.plug(Some(HIGH_POWER)).await.unwrap();

    // Plug in device 0 and disable sink, device 1 should remain current sink
    // Device 0 should not be able to sink after device 1 is unplugged
    info!("Connecting device 0");
    device0.plug(Some(LOW_POWER)).await.unwrap();
    device0.disable_sink().await.unwrap();
    device1.unplug().await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy::task());
        spawner.must_spawn(run(spawner));
    });
}
