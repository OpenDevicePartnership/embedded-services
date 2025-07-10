//! Low-level example of external messaging with a simple type-C service
use embassy_executor::{Executor, Spawner};
use embedded_services::type_c::{ControllerId, external};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;

#[embassy_executor::task]
async fn task(_spawner: Spawner) {
    info!("Starting main task");
    embedded_services::init().await;

    info!("Getting controller status");
    let controller_status = external::get_controller_status(ControllerId(0)).await.unwrap();
    info!("Controller status: {controller_status:?}");

    info!("Getting port status");
    let port_status = external::get_port_status(GlobalPortId(0)).await.unwrap();
    info!("Port status: {port_status:?}");

    info!("Getting retimer fw update status");
    let rt_fw_update_status = external::port_get_rt_fw_update_status(GlobalPortId(0)).await.unwrap();
    info!("Get retimer fw update status: {rt_fw_update_status:?}");

    info!("Setting retimer fw update state");
    external::port_set_rt_fw_update_state(GlobalPortId(0)).await.unwrap();

    info!("Clearing retimer fw update state");
    external::port_clear_rt_fw_update_state(GlobalPortId(0)).await.unwrap();

    info!("Setting retimer compliance");
    external::port_set_rt_compliance(GlobalPortId(0)).await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(type_c_service::task());
        spawner.must_spawn(task(spawner));
    });
}
