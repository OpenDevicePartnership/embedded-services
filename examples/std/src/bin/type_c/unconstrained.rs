use embassy_executor::{Executor, Spawner};
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::power::policy::PowerCapability;
use embedded_services::power::{self, policy};
use embedded_services::type_c::{ControllerId, controller};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::wrapper::backing::BackingDefaultStorage;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: GlobalPortId = GlobalPortId(0);
const POWER0: power::policy::DeviceId = power::policy::DeviceId(0);
const CFU0: u8 = 0x00;

const CONTROLLER1: ControllerId = ControllerId(1);
const PORT1: GlobalPortId = GlobalPortId(1);
const POWER1: power::policy::DeviceId = power::policy::DeviceId(1);
const CFU1: u8 = 0x01;

const CONTROLLER2: ControllerId = ControllerId(2);
const PORT2: GlobalPortId = GlobalPortId(2);
const POWER2: power::policy::DeviceId = power::policy::DeviceId(2);
const CFU2: u8 = 0x02;

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

    static BACKING_STORAGE0: StaticCell<BackingDefaultStorage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage0 = BACKING_STORAGE0.init(BackingDefaultStorage::new());
    let backing0 = backing_storage0
        .get_backing()
        .expect("Failed to create backing storage");

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    let controller0 = mock_controller::Controller::new(state0);
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(mock_controller::Wrapper::new(
        embedded_services::type_c::controller::Device::new(CONTROLLER0, &[PORT0]),
        [policy::device::Device::new(POWER0)],
        embedded_services::cfu::component::CfuDevice::new(CFU0),
        backing0,
        controller0,
        crate::mock_controller::Validator,
    ));

    static BACKING_STORAGE1: StaticCell<BackingDefaultStorage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage1 = BACKING_STORAGE1.init(BackingDefaultStorage::new());
    let backing1 = backing_storage1
        .get_backing()
        .expect("Failed to create backing storage");

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    let controller1 = mock_controller::Controller::new(state1);
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(mock_controller::Wrapper::new(
        embedded_services::type_c::controller::Device::new(CONTROLLER1, &[PORT1]),
        [policy::device::Device::new(POWER1)],
        embedded_services::cfu::component::CfuDevice::new(CFU1),
        backing1,
        controller1,
        crate::mock_controller::Validator,
    ));

    static BACKING_STORAGE2: StaticCell<BackingDefaultStorage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage2 = BACKING_STORAGE2.init(BackingDefaultStorage::new());
    let backing2 = backing_storage2
        .get_backing()
        .expect("Failed to create backing storage");

    static STATE2: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state2 = STATE2.init(mock_controller::ControllerState::new());
    let controller2 = mock_controller::Controller::new(state2);
    static WRAPPER2: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper2 = WRAPPER2.init(mock_controller::Wrapper::new(
        embedded_services::type_c::controller::Device::new(CONTROLLER2, &[PORT2]),
        [policy::device::Device::new(POWER2)],
        embedded_services::cfu::component::CfuDevice::new(CFU2),
        backing2,
        controller2,
        crate::mock_controller::Validator,
    ));

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
