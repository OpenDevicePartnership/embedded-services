use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, Receiver as ChannelReceiver, Sender as ChannelSender};
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::{info, warn};
use static_cell::StaticCell;
use thermal_service as ts;
use thermal_service_interface::ThermalService;
use thermal_service_interface::fan::FanService;
use thermal_service_interface::sensor;
use thermal_service_interface::sensor::SensorService;

// More readable type aliases for sensor, fan, and thermal services used in this example
type MockSensorService = ts::sensor::Service<
    'static,
    ts::mock::sensor::MockSensor,
    ChannelSender<'static, GlobalRawMutex, sensor::Event, 4>,
    16,
>;
type MockFanService =
    ts::fan::Service<'static, ts::mock::fan::MockFan, MockSensorService, embedded_services::event::NoopSender, 16>;
type MockThermalService = ts::Service<'static, MockSensorService, MockFanService>;

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    // Create a backing channel for sensor events to be sent on
    static SENSOR_EVENT_CHANNEL: StaticCell<Channel<GlobalRawMutex, sensor::Event, 4>> = StaticCell::new();
    let sensor_event_channel = SENSOR_EVENT_CHANNEL.init(Channel::new());

    // Then create the list of senders for the sensor service to use
    // Though we are only using one sender in this example, an abitrary number could be used
    static SENSOR_SENDERS: StaticCell<[ChannelSender<'static, GlobalRawMutex, sensor::Event, 4>; 1]> =
        StaticCell::new();
    let event_senders = SENSOR_SENDERS.init([sensor_event_channel.sender()]);

    // Spawn the sensor service which will begin running and generating events
    let sensor_service = odp_service_common::spawn_service!(
        spawner,
        MockSensorService,
        ts::sensor::InitParams {
            driver: ts::mock::sensor::MockSensor::new(),
            config: ts::mock::sensor::MockSensor::config(),
            event_senders,
        }
    )
    .expect("Failed to spawn sensor service");

    // Spawn the fan service which uses the above sensor service for automatic speed control
    // In this example, we use an empty event sender list since the fan won't generate any events
    let fan_service = odp_service_common::spawn_service!(
        spawner,
        MockFanService,
        ts::fan::InitParams {
            driver: ts::mock::fan::MockFan::new(),
            config: ts::mock::fan::MockFan::config(),
            sensor_service,
            event_senders: &mut [],
        }
    )
    .expect("Failed to spawn fan service");

    // The thermal service accepts slices of associated sensors and fans,
    // so we need static lifetime here since the thermal service handle is passed to task
    static SENSORS: StaticCell<[MockSensorService; 1]> = StaticCell::new();
    let sensors = SENSORS.init([sensor_service]);

    static FANS: StaticCell<[MockFanService; 1]> = StaticCell::new();
    let fans = FANS.init([fan_service]);

    // The thermal service handle mainly exists for host relaying, but this example does not make use of that
    //
    // However, we can still use the thermal service handle to access registered sensors and fans by id
    static RESOURCES: StaticCell<ts::Resources<MockSensorService, MockFanService>> = StaticCell::new();
    let resources = RESOURCES.init(ts::Resources::default());
    let thermal_service = ts::Service::init(resources, ts::InitParams { sensors, fans });

    spawner.spawn(monitor(thermal_service).expect("Failed to create monitor task"));
    spawner.spawn(
        sensor_event_listener(sensor_event_channel.receiver()).expect("Failed to create sensor event listener task"),
    );
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(run(spawner).expect("Failed to create run task"));
    });
}

#[embassy_executor::task]
async fn sensor_event_listener(receiver: ChannelReceiver<'static, GlobalRawMutex, sensor::Event, 4>) {
    loop {
        let event = receiver.receive().await;
        warn!("Sensor event: {:?}", event);
    }
}

#[embassy_executor::task]
async fn monitor(service: MockThermalService) {
    loop {
        if let Some(sensor) = service.sensor(0) {
            let temp = sensor.temperature().await;
            info!("Mock sensor temp: {} C", temp);
        }

        if let Some(fan) = service.fan(0) {
            let rpm = fan.rpm().await;
            info!("Mock fan RPM: {}", rpm);
        }

        Timer::after_secs(1).await;
    }
}
