use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::power::policy::PowerCapability;
use embedded_services::power::{self};
use embedded_services::type_c::{ControllerId, controller};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power::policy::DeviceId = power::policy::DeviceId(0);
const CFU0_ID: u8 = 0x00;

const CONTROLLER1_ID: ControllerId = ControllerId(1);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const POWER1_ID: power::policy::DeviceId = power::policy::DeviceId(1);
const CFU1_ID: u8 = 0x01;

const CONTROLLER2_ID: ControllerId = ControllerId(2);
const PORT2_ID: GlobalPortId = GlobalPortId(2);
const POWER2_ID: power::policy::DeviceId = power::policy::DeviceId(2);
const CFU2_ID: u8 = 0x02;

const DELAY_MS: u64 = 1000;

#[embassy_executor::task(pool_size = 3)]
async fn controller_task(wrapper: &'static mock_controller::Wrapper<'static>) {
    wrapper.register().await.unwrap();

    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    controller::init();

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(CONTROLLER0_ID, CFU0_ID, [(PORT0_ID, POWER0_ID)]));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    static CONTROLLER0: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller0 = CONTROLLER0.init(Mutex::new(mock_controller::Controller::new(state0)));
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(
        mock_controller::Wrapper::try_new(controller0, referenced, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(CONTROLLER1_ID, CFU1_ID, [(PORT1_ID, POWER1_ID)]));
    static REFERENCED1: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced1 = REFERENCED1.init(storage1.create_referenced());

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    static CONTROLLER1: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller1 = CONTROLLER1.init(Mutex::new(mock_controller::Controller::new(state1)));
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(
        mock_controller::Wrapper::try_new(controller1, referenced1, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    static STORAGE2: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage2 = STORAGE2.init(Storage::new(CONTROLLER2_ID, CFU2_ID, [(PORT2_ID, POWER2_ID)]));
    static REFERENCED2: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced2 = REFERENCED2.init(storage2.create_referenced());

    static STATE2: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state2 = STATE2.init(mock_controller::ControllerState::new());
    static CONTROLLER2: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller2 = CONTROLLER2.init(Mutex::new(mock_controller::Controller::new(state2)));
    static WRAPPER2: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper2 = WRAPPER2.init(
        mock_controller::Wrapper::try_new(controller2, referenced2, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    info!("Starting controller tasks");
    spawner.must_spawn(controller_task(wrapper0));
    spawner.must_spawn(controller_task(wrapper1));
    spawner.must_spawn(controller_task(wrapper2));

    const CAPABILITY: PowerCapability = PowerCapability {
        voltage_mv: 20000,
        current_ma: 5000,
    };

    // Wait for controller to be registered
    Timer::after_secs(1).await;

    info!("Connecting port 0, unconstrained");
    state0.connect_sink(CAPABILITY, true).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Connecting port 1, constrained");
    state1.connect_sink(CAPABILITY, false).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Disconnecting port 0");
    state0.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Disconnecting port 1");
    state1.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Connecting port 0, unconstrained");
    state0.connect_sink(CAPABILITY, true).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Connecting port 1, unconstrained");
    state1.connect_sink(CAPABILITY, true).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Connecting port 2, unconstrained");
    state2.connect_sink(CAPABILITY, true).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Disconnecting port 0");
    state0.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Disconnecting port 1");
    state1.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Disconnecting port 2");
    state2.disconnect().await;
    Timer::after_millis(DELAY_MS).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task(Default::default()));
        spawner.must_spawn(type_c_service::task(Default::default()));
        spawner.must_spawn(task(spawner));
    });
}
