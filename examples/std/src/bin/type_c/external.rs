//! Low-level example of external messaging with a simple type-C service
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller::{self, Wrapper};
use type_c_service::service::{Service, config::Config};
use type_c_service::type_c::{Cached, ControllerId, controller, controller::Context};
use type_c_service::wrapper::backing::Storage;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power_policy_service::psu::DeviceId = power_policy_service::psu::DeviceId(0);

#[embassy_executor::task]
async fn controller_task(wrapper: &'static Wrapper<'static>) {
    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(_spawner: Spawner, controller_context: &'static Context) {
    info!("Starting main task");
    embedded_services::init().await;

    // Allow the controller to initialize and register itself
    Timer::after_secs(1).await;
    info!("Getting controller status");
    let controller_status = controller_context
        .get_controller_status_external(ControllerId(0))
        .await
        .unwrap();
    info!("Controller status: {controller_status:?}");

    info!("Getting port status");
    let port_status = controller_context
        .get_port_status_external(GlobalPortId(0), Cached(true))
        .await
        .unwrap();
    info!("Port status: {port_status:?}");

    info!("Getting retimer fw update status");
    let rt_fw_update_status = controller_context
        .port_get_rt_fw_update_status_external(GlobalPortId(0))
        .await
        .unwrap();
    info!("Get retimer fw update status: {rt_fw_update_status:?}");

    info!("Setting retimer fw update state");
    controller_context
        .port_set_rt_fw_update_state_external(GlobalPortId(0))
        .await
        .unwrap();

    info!("Clearing retimer fw update state");
    controller_context
        .port_clear_rt_fw_update_state_external(GlobalPortId(0))
        .await
        .unwrap();

    info!("Setting retimer compliance");
    controller_context
        .port_set_rt_compliance_external(GlobalPortId(0))
        .await
        .unwrap();

    info!("Setting max sink voltage");
    controller_context
        .set_max_sink_voltage_external(GlobalPortId(0), Some(5000))
        .await
        .unwrap();

    info!("Clearing dead battery flag");
    controller_context
        .clear_dead_battery_flag_external(GlobalPortId(0))
        .await
        .unwrap();

    info!("Reconfiguring retimer");
    controller_context
        .reconfigure_retimer_external(GlobalPortId(0))
        .await
        .unwrap();
}

#[embassy_executor::task]
async fn service_task(
    controller_context: &'static Context,
    controllers: &'static IntrusiveList,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_service::service::event::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    for controller_wrapper in wrappers {
        controller::register_controller(controllers, controller_wrapper.registration.pd_controller).unwrap();
    }

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(Service::create(
        Config::default(),
        controller_context,
        controllers,
        power_policy_publisher,
        power_policy_subscriber,
    ));

    loop {
        if let Err(e) = service.process_next_event().await {
            error!("Type-C service processing error: {e:#?}");
        }
    }
}

fn create_wrapper(controller_context: &'static Context) -> &'static mut Wrapper<'static> {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage = STORAGE.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [PORT0_ID],
    ));

    static INTERMEDIATE: StaticCell<type_c_service::wrapper::backing::IntermediateStorage<1, GlobalRawMutex>> =
        StaticCell::new();
    let intermediate = INTERMEDIATE.init(
        backing_storage
            .try_create_intermediate()
            .expect("Failed to create intermediate storage"),
    );

    static POLICY_CHANNEL: StaticCell<Channel<GlobalRawMutex, power_policy_service::psu::event::RequestData, 1>> =
        StaticCell::new();
    let policy_channel = POLICY_CHANNEL.init(Channel::new());

    let policy_sender = policy_channel.dyn_sender();
    let policy_receiver = policy_channel.dyn_receiver();

    static REFERENCED: StaticCell<
        type_c_service::wrapper::backing::ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_service::psu::event::RequestData>,
            DynamicReceiver<'_, power_policy_service::psu::event::RequestData>,
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
    WRAPPER.init(
        mock_controller::Wrapper::try_new(
            controller,
            Default::default(),
            referenced,
            crate::mock_controller::Validator,
        )
        .expect("Failed to create wrapper"),
    )
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let context = CONTEXT.init(type_c_service::type_c::controller::Context::new());

    let wrapper = create_wrapper(context);

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(service_task(context, controller_list, [wrapper]));
        spawner.must_spawn(task(spawner, context));
        spawner.must_spawn(controller_task(wrapper));
    });
}
