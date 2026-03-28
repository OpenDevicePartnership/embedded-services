use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::{error, info};
use static_cell::StaticCell;
use thermal_service as ts;

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    static SENSOR: StaticCell<ts::mock::TsMockSensor> = StaticCell::new();
    let sensor = SENSOR.init(ts::mock::new_sensor());

    static FAN: StaticCell<ts::mock::TsMockFan> = StaticCell::new();
    let fan = FAN.init(ts::mock::new_fan());

    static SENSORS: StaticCell<[&'static ts::sensor::Device; 1]> = StaticCell::new();
    let sensors = SENSORS.init([sensor.device()]);

    static FANS: StaticCell<[&'static ts::fan::Device; 1]> = StaticCell::new();
    let fans = FANS.init([fan.device()]);

    static STORAGE: OnceLock<ts::Service<'static>> = OnceLock::new();
    let thermal_service = ts::Service::init(&STORAGE, sensors, fans).await;

    let _fan_service = odp_service_common::spawn_service!(
        spawner,
        ts::fan::Service<'static, ts::mock::fan::MockFan, 16>,
        ts::fan::InitParams { fan, thermal_service }
    )
    .expect("Failed to spawn fan service");

    let _sensor_service = odp_service_common::spawn_service!(
        spawner,
        ts::sensor::Service<'static, ts::mock::sensor::MockSensor, 16>,
        ts::sensor::InitParams {
            sensor,
            thermal_service
        }
    )
    .expect("Failed to spawn sensor service");

    spawner.must_spawn(monitor(thermal_service));
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}

#[embassy_executor::task]
async fn monitor(service: &'static ts::Service<'static>) {
    loop {
        match service
            .execute_sensor_request(ts::mock::MOCK_SENSOR_ID, ts::sensor::Request::GetTemp)
            .await
        {
            Ok(ts::sensor::ResponseData::Temp(temp)) => info!("Mock sensor temp: {} C", temp),
            _ => error!("Failed to monitor mock sensor temp"),
        }
        match service
            .execute_fan_request(ts::mock::MOCK_FAN_ID, ts::fan::Request::GetRpm)
            .await
        {
            Ok(ts::fan::ResponseData::Rpm(rpm)) => info!("Mock fan RPM: {}", rpm),
            _ => error!("Failed to monitor mock fan RPM"),
        }

        Timer::after_secs(1).await;
    }
}
