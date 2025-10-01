//! Low-level example of external messaging with a simple type-C service
use embassy_executor::{Executor, Spawner};
use embassy_time::Timer;
use embedded_services::{
    GlobalRawMutex, power,
    type_c::{Cached, ControllerId, external},
};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::wrapper::backing::Storage;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: GlobalPortId = GlobalPortId(0);
const POWER0: power::policy::DeviceId = power::policy::DeviceId(0);

#[embassy_executor::task]
async fn controller_task() {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage = STORAGE.init(Storage::new(
        CONTROLLER0,
        0, // CFU component ID (unused)
        [(PORT0, POWER0)],
    ));
    static REFERENCED: StaticCell<type_c_service::wrapper::backing::ReferencedStorage<1, GlobalRawMutex>> =
        StaticCell::new();
    let referenced = REFERENCED.init(backing_storage.create_referenced());

    static WRAPPER: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let controller = mock_controller::Controller::new(state);
    let wrapper = WRAPPER.init(
        mock_controller::Wrapper::try_new(controller, referenced, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    wrapper.register().await.unwrap();
    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(_spawner: Spawner) {
    info!("Starting main task");
    embedded_services::init().await;

    // Allow the controller to initialize and register itself
    Timer::after_secs(1).await;
    info!("Getting controller status");
    let controller_status = external::get_controller_status(ControllerId(0)).await.unwrap();
    info!("Controller status: {controller_status:?}");

    info!("Getting port status");
    let port_status = external::get_port_status(GlobalPortId(0), Cached(true)).await.unwrap();
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

    info!("Setting max sink voltage");
    external::set_max_sink_voltage(GlobalPortId(0), Some(5000))
        .await
        .unwrap();

    info!("Clearing dead battery flag");
    external::clear_dead_battery_flag(GlobalPortId(0)).await.unwrap();

    info!("Reconfiguring retimer");
    external::reconfigure_retimer(GlobalPortId(0)).await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(type_c_service::task(Default::default()));
        spawner.must_spawn(task(spawner));
        spawner.must_spawn(controller_task());
    });
}
