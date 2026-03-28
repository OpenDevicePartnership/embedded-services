//! Standard battery example
//!
//! The example can be run simply by typing `cargo run --bin battery`

use battery_service as bs;
use embassy_executor::{Executor, Spawner};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use odp_service_common::runnable_service::spawn_service;

#[embassy_executor::task]
async fn embassy_main(spawner: Spawner) {
    embedded_services::debug!("Initializing battery service");
    embedded_services::init().await;

    static BATTERY_DEVICE: StaticCell<bs::device::Device> = StaticCell::new();
    let device = BATTERY_DEVICE.init(bs::device::Device::new(Default::default()));

    let battery_service = spawn_service!(
        spawner,
        battery_service::Service<'static, 1>,
        battery_service::InitParams {
            config: Default::default(),
            devices: [device],
        }
    )
    .expect("Failed to initialize battery service");

    static BATTERY_WRAPPER: StaticCell<bs::mock::MockBattery> = StaticCell::new();
    let wrapper = BATTERY_WRAPPER.init(bs::wrapper::Wrapper::new(
        device,
        battery_service::mock::MockBatteryDriver::new(),
    ));

    #[embassy_executor::task]
    async fn battery_wrapper_process(battery_wrapper: &'static battery_service::mock::MockBattery<'static>) {
        battery_wrapper.process().await
    }

    spawner.must_spawn(battery_wrapper_process(wrapper));
    spawner.must_spawn(run_app(battery_service));
}

#[embassy_executor::task]
pub async fn run_app(battery_service: battery_service::Service<'static, 1>) {
    // Initialize battery state machine.
    let mut retries = 5;
    while let Err(e) = bs::mock::init_state_machine(&battery_service).await {
        retries -= 1;
        if retries <= 0 {
            embedded_services::error!("Failed to initialize Battery: {:?}", e);
            return;
        }
        Timer::after(Duration::from_secs(1)).await;
    }

    let mut failures: u32 = 0;
    let mut count: usize = 1;
    loop {
        Timer::after(Duration::from_secs(1)).await;
        if count.is_multiple_of(const { 60 * 60 * 60 }) {
            if let Err(e) = battery_service
                .execute_event(battery_service::context::BatteryEvent {
                    event: battery_service::context::BatteryEventInner::PollStaticData,
                    device_id: bs::device::DeviceId(0),
                })
                .await
            {
                failures += 1;
                embedded_services::error!("Fuel gauge static data error: {:#?}", e);
            }
        }
        if let Err(e) = battery_service
            .execute_event(battery_service::context::BatteryEvent {
                event: battery_service::context::BatteryEventInner::PollDynamicData,
                device_id: bs::device::DeviceId(0),
            })
            .await
        {
            failures += 1;
            embedded_services::error!("Fuel gauge dynamic data error: {:#?}", e);
        }

        if failures > 10 {
            failures = 0;
            count = 0;
            embedded_services::error!("FG: Too many errors, timing out and starting recovery...");
            if bs::mock::recover_state_machine(&battery_service).await.is_err() {
                embedded_services::error!("FG: Fatal error");
                return;
            }
        }

        count = count.wrapping_add(1);
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();
    embedded_services::info!("battery example started");

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    // Run battery service
    executor.run(|spawner| {
        spawner.must_spawn(embassy_main(spawner));
    });
}
