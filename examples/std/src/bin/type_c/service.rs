use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::event::MapSender;
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::type_c::Current;
use log::*;
use power_policy_interface::charger::mock::ChargerType;
use power_policy_interface::psu;
use power_policy_service::psu::PsuEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use std_examples::type_c::mock_controller::Wrapper;
use type_c_interface::port::{ControllerId, PortRegistration};
use type_c_interface::service::context::Context;
use type_c_interface::service::event::PortEvent as ServicePortEvent;
use type_c_service::service::config::Config;
use type_c_service::service::{EventReceiver, Service};
use type_c_service::util::power_capability_from_current;
use type_c_service::wrapper::backing::Storage;
use type_c_service::wrapper::event_receiver::ArrayPortEventReceivers;
use type_c_service::wrapper::message::*;
use type_c_service::wrapper::proxy::PowerProxyDevice;

const NUM_PD_CONTROLLERS: usize = 1;
const CHANNEL_CAPACITY: usize = 4;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const DELAY_MS: u64 = 1000;

type DeviceType = Mutex<GlobalRawMutex, PowerProxyDevice<'static>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, DeviceType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, DeviceType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;

type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, DeviceType, 1, PowerPolicySenderType, 1, ChargerType, 0>,
    >,
>;

type ServiceType = Service<'static>;

#[embassy_executor::task]
async fn controller_task(
    mut event_receiver: ArrayPortEventReceivers<'static, 1, mock_controller::InterruptReceiver<'static>>,
    wrapper: &'static Wrapper<'static>,
    controller: &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
) {
    controller.lock().await.custom_function();

    loop {
        let event = event_receiver.wait_event().await;

        let output = wrapper
            .process_event(
                &mut event_receiver.sink_ready_timeout,
                &mut event_receiver.cfu_event_receiver,
                event,
            )
            .await;
        if let Err(e) = output {
            error!("Error processing event: {e:?}");
        }

        let output = output.unwrap();
        if let Output::PdAlert(OutputPdAlert { port, ado }) = &output {
            info!("Port{}: PD alert received: {:?}", port.0, ado);
        }

        if let Err(e) = wrapper.finalize(&mut event_receiver.power_proxies, output).await {
            error!("Error finalizing output: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    // Create power policy service
    static CONTEXT: StaticCell<type_c_interface::service::context::Context> = StaticCell::new();
    let controller_context = CONTEXT.init(type_c_interface::service::context::Context::new());

    let (event_receiver, wrapper, policy_receiver, controller, state) = create_wrapper(controller_context);

    // Create type-c service
    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_interface::service::event::EventData, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_sender: PowerPolicySenderType =
        MapSender::new(power_policy_channel.dyn_immediate_publisher(), |e| e.into());
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let power_policy_registration = ArrayRegistration {
        psus: [&wrapper.ports[0].proxy],
        service_senders: [power_policy_sender],
        chargers: [],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_policy_service::service::config::Config::default(),
    )));

    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, ServiceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Mutex::new(Service::create(Config::default(), controller_context)));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    spawner.spawn(
        power_policy_psu_task(
            PsuEventReceivers::new([&wrapper.ports[0].proxy], [policy_receiver]),
            power_service,
        )
        .expect("Failed to create power policy task"),
    );
    spawner.spawn(
        type_c_service_task(
            type_c_service,
            EventReceiver::new(controller_context, power_policy_subscriber),
            [wrapper],
            cfu_client,
        )
        .expect("Failed to create type-c service task"),
    );
    spawner.spawn(controller_task(event_receiver, wrapper, controller).expect("Failed to create controller task"));

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
async fn power_policy_psu_task(
    psu_events: PsuEventReceivers<'static, 1, DeviceType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::psu_task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, ServiceType>,
    event_receiver: EventReceiver<'static, PowerPolicyReceiverType>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, event_receiver, wrappers, cfu_client).await;
}

fn create_wrapper(
    context: &'static Context,
) -> (
    ArrayPortEventReceivers<'static, 1, mock_controller::InterruptReceiver<'static>>,
    &'static Wrapper<'static>,
    DynamicReceiver<'static, power_policy_interface::psu::event::EventData>,
    &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
    &'static mock_controller::ControllerState,
) {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static PORT0_CHANNEL: Channel<GlobalRawMutex, ServicePortEvent, CHANNEL_CAPACITY> = Channel::new();

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [PortRegistration {
            id: PORT0_ID,
            sender: PORT0_CHANNEL.dyn_sender(),
            receiver: PORT0_CHANNEL.dyn_receiver(),
        }],
    ));

    static POLICY_CHANNEL: StaticCell<Channel<GlobalRawMutex, power_policy_interface::psu::event::EventData, 1>> =
        StaticCell::new();
    let policy_channel = POLICY_CHANNEL.init(Channel::new());

    let policy_sender = policy_channel.dyn_sender();
    let policy_receiver = policy_channel.dyn_receiver();

    let (intermediate, power_event_receivers) = storage
        .try_create_intermediate([("Pd0", policy_sender)])
        .expect("Failed to create intermediate storage");

    static INTERMEDIATE: StaticCell<
        type_c_service::wrapper::backing::IntermediateStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'static, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let intermediate = INTERMEDIATE.init(intermediate);

    static REFERENCED: StaticCell<
        type_c_service::wrapper::backing::ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let referenced = REFERENCED.init(
        intermediate
            .try_create_referenced()
            .expect("Failed to create referenced storage"),
    );

    let event_receiver = ArrayPortEventReceivers::new(
        state.create_interrupt_receiver(),
        power_event_receivers,
        &referenced.pd_controller,
        &storage.cfu_device,
    );

    static CONTROLLER: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller = CONTROLLER.init(Mutex::new(mock_controller::Controller::new(state)));

    static WRAPPER: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    (
        event_receiver,
        WRAPPER.init(mock_controller::Wrapper::new(
            controller,
            Default::default(),
            referenced,
            crate::mock_controller::Validator,
        )),
        policy_receiver,
        controller,
        state,
    )
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(task(spawner).expect("Failed to create task"));
    });
}
