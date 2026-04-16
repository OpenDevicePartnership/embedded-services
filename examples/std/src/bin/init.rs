use embassy_executor::Executor;
use embassy_time::{self as _, Timer};
use embedded_services::init;
use log::*;
use static_cell::StaticCell;

#[embassy_executor::task]
async fn registration_waiter() {
    info!("Waiting for registration");
    init::wait_for_registration().await;
    info!("Registration done");
}

#[embassy_executor::task]
async fn registration_task() {
    info!("Registration task started");
    Timer::after(embassy_time::Duration::from_secs(1)).await;
    init::registration_done();
    info!("Registration task finished");
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(registration_waiter().unwrap());
        spawner.spawn(registration_task().unwrap());
    });
}
