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

    static SERVICE: OnceLock<ts::Service> = OnceLock::new();
    let service = ts::Service::new(&SERVICE, &[sensor.device()], &[fan.device()])
        .await
        .expect("Failed to initialize thermal service");

    spawner.must_spawn(sensor_task(service, sensor));
    spawner.must_spawn(fan_task(service, fan));
    spawner.must_spawn(handle_requests_task(service));
    spawner.must_spawn(monitor(service));
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
async fn sensor_task(service: &'static ts::Service, sensor: &'static ts::mock::TsMockSensor) {
    ts::task::sensor_task(sensor, service).await
}

#[embassy_executor::task]
async fn fan_task(service: &'static ts::Service, fan: &'static ts::mock::TsMockFan) {
    ts::task::fan_task(fan, service).await;
}

#[embassy_executor::task]
async fn handle_requests_task(service: &'static ts::Service) {
    ts::task::handle_requests(service).await;
}

#[embassy_executor::task]
async fn monitor(service: &'static ts::Service) {
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
