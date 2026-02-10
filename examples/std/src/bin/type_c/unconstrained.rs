use crate::mock_controller::Wrapper;
use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::Channel;
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::channel::DynamicSender;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_usb_pd::GlobalPortId;
use log::*;
use power_policy_service::capability::PowerCapability;
use power_policy_service::service::PowerPolicy;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::service::Service;
use type_c_service::type_c::ControllerId;

const NUM_PD_CONTROLLERS: usize = 3;
use type_c_service::wrapper::backing::{IntermediateStorage, ReferencedStorage, Storage};
use type_c_service::wrapper::proxy::PowerProxyDevice;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power_policy_service::device::DeviceId = power_policy_service::device::DeviceId(0);
const CFU0_ID: u8 = 0x00;

const CONTROLLER1_ID: ControllerId = ControllerId(1);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const POWER1_ID: power_policy_service::device::DeviceId = power_policy_service::device::DeviceId(1);
const CFU1_ID: u8 = 0x01;

const CONTROLLER2_ID: ControllerId = ControllerId(2);
const PORT2_ID: GlobalPortId = GlobalPortId(2);
const POWER2_ID: power_policy_service::device::DeviceId = power_policy_service::device::DeviceId(2);
const CFU2_ID: u8 = 0x02;

const DELAY_MS: u64 = 1000;

#[embassy_executor::task(pool_size = 3)]
async fn controller_task(wrapper: &'static mock_controller::Wrapper<'static>) {
    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    // Create power policy service
    static POWER_SERVICE_CONTEXT: StaticCell<
        power_policy_service::service::context::Context<
            Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
            DynamicReceiver<'static, power_policy_service::device::event::RequestData>,
        >,
    > = StaticCell::new();
    let power_service_context = POWER_SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

    static CONTROLLER_CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTROLLER_CONTEXT.init(type_c_service::type_c::controller::Context::new());

    static POWER_SERVICE: StaticCell<
        power_policy_service::service::PowerPolicy<
            Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
            DynamicReceiver<'static, power_policy_service::device::event::RequestData>,
        >,
    > = StaticCell::new();
    let power_service = POWER_SERVICE.init(power_policy_service::service::PowerPolicy::new(
        power_service_context,
        power_policy_service::service::config::Config::default(),
    ));

    static CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let context = CONTEXT.init(type_c_service::type_c::controller::Context::new());
    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    static STORAGE0: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage0 = STORAGE0.init(Storage::new(context, CONTROLLER0_ID, CFU0_ID, [PORT0_ID]));
    static INTERMEDIATE0: StaticCell<IntermediateStorage<1, GlobalRawMutex>> = StaticCell::new();
    let intermediate0 = INTERMEDIATE0.init(
        storage0
            .try_create_intermediate()
            .expect("Failed to create intermediate storage"),
    );

    static POLICY_CHANNEL0: StaticCell<Channel<GlobalRawMutex, power_policy_service::device::event::RequestData, 1>> =
        StaticCell::new();
    let policy_channel0 = POLICY_CHANNEL0.init(Channel::new());
    let policy_sender0 = policy_channel0.dyn_sender();
    let policy_receiver0 = policy_channel0.dyn_receiver();

    static REFERENCED0: StaticCell<
        ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_service::device::event::RequestData>,
            DynamicReceiver<'_, power_policy_service::device::event::RequestData>,
        >,
    > = StaticCell::new();
    let referenced0 = REFERENCED0.init(
        intermediate0
            .try_create_referenced([(POWER0_ID, policy_sender0, policy_receiver0)])
            .expect("Failed to create referenced storage"),
    );

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    static CONTROLLER0: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller0 = CONTROLLER0.init(Mutex::new(mock_controller::Controller::new(state0)));
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(
        mock_controller::Wrapper::try_new(
            controller0,
            Default::default(),
            referenced0,
            crate::mock_controller::Validator,
        )
        .expect("Failed to create wrapper"),
    );

    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(context, CONTROLLER1_ID, CFU1_ID, [PORT1_ID]));
    static INTERMEDIATE1: StaticCell<IntermediateStorage<1, GlobalRawMutex>> = StaticCell::new();
    let intermediate1 = INTERMEDIATE1.init(
        storage1
            .try_create_intermediate()
            .expect("Failed to create intermediate storage"),
    );

    static POLICY_CHANNEL1: StaticCell<Channel<GlobalRawMutex, power_policy_service::device::event::RequestData, 1>> =
        StaticCell::new();
    let policy_channel1 = POLICY_CHANNEL1.init(Channel::new());
    let policy_sender1 = policy_channel1.dyn_sender();
    let policy_receiver1 = policy_channel1.dyn_receiver();

    static REFERENCED1: StaticCell<
        ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_service::device::event::RequestData>,
            DynamicReceiver<'_, power_policy_service::device::event::RequestData>,
        >,
    > = StaticCell::new();
    let referenced1 = REFERENCED1.init(
        intermediate1
            .try_create_referenced([(POWER1_ID, policy_sender1, policy_receiver1)])
            .expect("Failed to create referenced storage"),
    );

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    static CONTROLLER1: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller1 = CONTROLLER1.init(Mutex::new(mock_controller::Controller::new(state1)));
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(
        mock_controller::Wrapper::try_new(
            controller1,
            Default::default(),
            referenced1,
            crate::mock_controller::Validator,
        )
        .expect("Failed to create wrapper"),
    );

    static STORAGE2: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage2 = STORAGE2.init(Storage::new(context, CONTROLLER2_ID, CFU2_ID, [PORT2_ID]));
    static INTERMEDIATE2: StaticCell<IntermediateStorage<1, GlobalRawMutex>> = StaticCell::new();
    let intermediate2 = INTERMEDIATE2.init(
        storage2
            .try_create_intermediate()
            .expect("Failed to create intermediate storage"),
    );

    static POLICY_CHANNEL2: StaticCell<Channel<GlobalRawMutex, power_policy_service::device::event::RequestData, 1>> =
        StaticCell::new();
    let policy_channel2 = POLICY_CHANNEL2.init(Channel::new());
    let policy_sender2 = policy_channel2.dyn_sender();
    let policy_receiver2 = policy_channel2.dyn_receiver();

    static REFERENCED2: StaticCell<
        ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_service::device::event::RequestData>,
            DynamicReceiver<'_, power_policy_service::device::event::RequestData>,
        >,
    > = StaticCell::new();
    let referenced2 = REFERENCED2.init(
        intermediate2
            .try_create_referenced([(POWER2_ID, policy_sender2, policy_receiver2)])
            .expect("Failed to create referenced storage"),
    );

    static STATE2: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state2 = STATE2.init(mock_controller::ControllerState::new());
    static CONTROLLER2: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller2 = CONTROLLER2.init(Mutex::new(mock_controller::Controller::new(state2)));
    static WRAPPER2: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper2 = WRAPPER2.init(
        mock_controller::Wrapper::try_new(
            controller2,
            Default::default(),
            referenced2,
            crate::mock_controller::Validator,
        )
        .expect("Failed to create wrapper"),
    );

    // Create type-c service
    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_service::service::event::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    static TYPE_C_SERVICE: StaticCell<Service<'static>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Service::create(
        Default::default(),
        controller_context,
        controller_list,
        power_policy_publisher,
        power_policy_subscriber,
    ));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    spawner.must_spawn(power_policy_service_task(power_service));
    spawner.must_spawn(type_c_service_task(
        type_c_service,
        [wrapper0, wrapper1, wrapper2],
        power_service_context,
        cfu_client,
    ));

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

#[embassy_executor::task]
async fn power_policy_service_task(
    service: &'static PowerPolicy<
        'static,
        Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
        DynamicReceiver<'static, power_policy_service::device::event::RequestData>,
    >,
) {
    power_policy_service::service::task::task(service)
        .await
        .expect("Failed to start power policy service task");
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Service<'static>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static power_policy_service::service::context::Context<
        Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
        DynamicReceiver<'static, power_policy_service::device::event::RequestData>,
    >,
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, wrappers, power_policy_context, cfu_client).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(task(spawner));
    });
}
