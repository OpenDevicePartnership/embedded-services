use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::type_c::Current;
use log::*;
use power_policy_service::PowerPolicy;
use power_policy_service::policy::policy;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use std_examples::type_c::mock_controller::Wrapper;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::type_c::controller::Context;
use type_c_service::type_c::{ControllerId, power_capability_from_current};
use type_c_service::wrapper::backing::Storage;
use type_c_service::wrapper::message::*;
use type_c_service::wrapper::proxy::PowerProxyDevice;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power_policy_service::policy::DeviceId = power_policy_service::policy::DeviceId(0);
const DELAY_MS: u64 = 1000;

#[embassy_executor::task]
async fn controller_task(
    wrapper: &'static Wrapper<'static>,
    controller: &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
) {
    controller.lock().await.custom_function();

    loop {
        let event = wrapper.wait_next().await;
        if let Err(e) = event {
            error!("Error waiting for event: {e:?}");
            continue;
        }
        let output = wrapper.process_event(event.unwrap()).await;
        if let Err(e) = output {
            error!("Error processing event: {e:?}");
        }

        let output = output.unwrap();
        if let Output::PdAlert(OutputPdAlert { port, ado }) = &output {
            info!("Port{}: PD alert received: {:?}", port.0, ado);
        }

        if let Err(e) = wrapper.finalize(output).await {
            error!("Error finalizing output: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    // Create power policy service
    static POWER_SERVICE_CONTEXT: StaticCell<
        power_policy_service::policy::policy::Context<
            Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
            DynamicReceiver<'static, policy::RequestData>,
        >,
    > = StaticCell::new();
    let power_service_context = POWER_SERVICE_CONTEXT.init(power_policy_service::policy::policy::Context::new());

    static CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTEXT.init(type_c_service::type_c::controller::Context::new());

    let (wrapper, controller, state) = create_wrapper(controller_context);

    static POWER_SERVICE: StaticCell<
        power_policy_service::PowerPolicy<
            Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
            DynamicReceiver<'static, policy::RequestData>,
        >,
    > = StaticCell::new();
    let power_service = POWER_SERVICE.init(power_policy_service::PowerPolicy::new(
        power_service_context,
        power_policy_service::config::Config::default(),
    ));

    // Create type-c service
    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_service::policy::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    static TYPE_C_SERVICE: StaticCell<Service<'static>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Service::create(
        Config::default(),
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
        [wrapper],
        power_service_context,
        cfu_client,
    ));
    spawner.must_spawn(controller_task(wrapper, controller));

    Timer::after_millis(1000).await;
    info!("Simulating connection");
    state
        .connect_sink(power_capability_from_current(Current::UsbDefault), false)
        .await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating PD alert");
    state.send_pd_alert(Ado::PowerButtonPress).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating disconnection");
    state.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating debug accessory connection");
    state.connect_debug_accessory_source(Current::UsbDefault).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating debug accessory disconnection");
    state.disconnect().await;
    Timer::after_millis(DELAY_MS).await;
}

#[embassy_executor::task]
async fn power_policy_service_task(
    service: &'static PowerPolicy<
        'static,
        Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
        DynamicReceiver<'static, policy::RequestData>,
    >,
) {
    power_policy_service::task::task(service)
        .await
        .expect("Failed to start power policy service task");
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Service<'static>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static policy::Context<
        Mutex<GlobalRawMutex, PowerProxyDevice<'static>>,
        DynamicReceiver<'static, policy::RequestData>,
    >,
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, wrappers, power_policy_context, cfu_client).await;
}

fn create_wrapper(
    context: &'static Context,
) -> (
    &'static Wrapper<'static>,
    &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
    &'static mock_controller::ControllerState,
) {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [PORT0_ID],
    ));

    static INTERMEDIATE: StaticCell<type_c_service::wrapper::backing::IntermediateStorage<1, GlobalRawMutex>> =
        StaticCell::new();
    let intermediate = INTERMEDIATE.init(
        storage
            .try_create_intermediate()
            .expect("Failed to create intermediate storage"),
    );

    static POLICY_CHANNEL: StaticCell<Channel<GlobalRawMutex, policy::RequestData, 1>> = StaticCell::new();
    let policy_channel = POLICY_CHANNEL.init(Channel::new());

    let policy_sender = policy_channel.dyn_sender();
    let policy_receiver = policy_channel.dyn_receiver();

    static REFERENCED: StaticCell<
        type_c_service::wrapper::backing::ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, policy::RequestData>,
            DynamicReceiver<'_, policy::RequestData>,
        >,
    > = StaticCell::new();
    let referenced = REFERENCED.init(
        intermediate
            .try_create_referenced([(POWER0_ID, policy_sender, policy_receiver)])
            .expect("Failed to create referenced storage"),
    );

    static CONTROLLER: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller = CONTROLLER.init(Mutex::new(mock_controller::Controller::new(state)));

    static WRAPPER: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    (
        WRAPPER.init(
            mock_controller::Wrapper::try_new(
                controller,
                Default::default(),
                referenced,
                crate::mock_controller::Validator,
            )
            .expect("Failed to create wrapper"),
        ),
        controller,
        state,
    )
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.must_spawn(task(spawner));
    });
}
