use embassy_executor::{Executor, Spawner};
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::power::policy::PowerCapability;
use embedded_services::power::{self};
use embedded_services::type_c::ControllerId;
use embedded_services::type_c::controller::Context;
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

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
async fn controller_task(wrapper: &'static mock_controller::Wrapper<'static>, controllers: &'static IntrusiveList) {
    wrapper.register(controllers).await.unwrap();

    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner, controller_context: &'static Context, controllers: &'static IntrusiveList) {
    embedded_services::init().await;

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(controller_context, CONTROLLER0, CFU0, [(PORT0, POWER0)]));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    let controller0 = mock_controller::Controller::new(state0);
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(
        mock_controller::Wrapper::try_new(controller0, referenced, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(controller_context, CONTROLLER1, CFU1, [(PORT1, POWER1)]));
    static REFERENCED1: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced1 = REFERENCED1.init(storage1.create_referenced());

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    let controller1 = mock_controller::Controller::new(state1);
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(
        mock_controller::Wrapper::try_new(controller1, referenced1, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    static STORAGE2: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage2 = STORAGE2.init(Storage::new(controller_context, CONTROLLER2, CFU2, [(PORT2, POWER2)]));
    static REFERENCED2: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced2 = REFERENCED2.init(storage2.create_referenced());

    static STATE2: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state2 = STATE2.init(mock_controller::ControllerState::new());
    let controller2 = mock_controller::Controller::new(state2);
    static WRAPPER2: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper2 = WRAPPER2.init(
        mock_controller::Wrapper::try_new(controller2, referenced2, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    info!("Starting controller tasks");
    spawner.must_spawn(controller_task(wrapper0, controllers));
    spawner.must_spawn(controller_task(wrapper1, controllers));
    spawner.must_spawn(controller_task(wrapper2, controllers));

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

#[embassy_executor::task]
async fn service_task(controller_context: &'static Context, controllers: &'static IntrusiveList) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<PubSubChannel<GlobalRawMutex, power::policy::CommsMessage, 4, 1, 0>> =
        StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let service = Service::create(
        Config::default(),
        controller_context,
        power_policy_publisher,
        power_policy_subscriber,
    );

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    if service.register_comms().await.is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

    if service.register_comms().await.is_err() {
        error!("Failed to register type-c service endpoint, service already registered?");
    }

    loop {
        let event = match service.wait_next(controllers).await {
            Ok(event) => event,
            Err(e) => {
                error!("Error waiting for next event: {:?}", e);
                continue;
            }
        };

        // Note: must call process_event before so port status is cached for everything else
        if let Err(e) = service.process_event(event, controllers).await {
            error!("Type-C service processing error: {:#?}", e);
        }
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let context = CONTEXT.init(embedded_services::type_c::controller::Context::new());
    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task(Default::default()));
        spawner.must_spawn(service_task(context, controller_list));
        spawner.must_spawn(task(spawner, context, controller_list));
    });
}
