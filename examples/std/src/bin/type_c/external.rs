//! Low-level example of external messaging with a simple type-C service
use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::{
    GlobalRawMutex, IntrusiveList, power,
    type_c::{Cached, ControllerId, controller::Context, external},
};
use embedded_usb_pd::GlobalPortId;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::service::{Service, config::Config};
use type_c_service::wrapper::backing::Storage;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power::policy::DeviceId = power::policy::DeviceId(0);

#[embassy_executor::task]
async fn controller_task(controller_context: &'static Context, controller_list: &'static IntrusiveList) {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let backing_storage = STORAGE.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [(PORT0_ID, POWER0_ID)],
    ));
    static REFERENCED: StaticCell<type_c_service::wrapper::backing::ReferencedStorage<1, GlobalRawMutex>> =
        StaticCell::new();
    let referenced = REFERENCED.init(backing_storage.create_referenced());

    static CONTROLLER: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller = CONTROLLER.init(Mutex::new(mock_controller::Controller::new(state)));

    static WRAPPER: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper = WRAPPER.init(
        mock_controller::Wrapper::try_new(
            controller,
            Default::default(),
            referenced,
            crate::mock_controller::Validator,
        )
        .expect("Failed to create wrapper"),
    );

    wrapper.register(controller_list).await.unwrap();
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
    let controller_status = external::get_controller_status(controller_context, ControllerId(0))
        .await
        .unwrap();
    info!("Controller status: {controller_status:?}");

    info!("Getting port status");
    let port_status = external::get_port_status(controller_context, GlobalPortId(0), Cached(true))
        .await
        .unwrap();
    info!("Port status: {port_status:?}");

    info!("Getting retimer fw update status");
    let rt_fw_update_status = external::port_get_rt_fw_update_status(controller_context, GlobalPortId(0))
        .await
        .unwrap();
    info!("Get retimer fw update status: {rt_fw_update_status:?}");

    info!("Setting retimer fw update state");
    external::port_set_rt_fw_update_state(controller_context, GlobalPortId(0))
        .await
        .unwrap();

    info!("Clearing retimer fw update state");
    external::port_clear_rt_fw_update_state(controller_context, GlobalPortId(0))
        .await
        .unwrap();

    info!("Setting retimer compliance");
    external::port_set_rt_compliance(controller_context, GlobalPortId(0))
        .await
        .unwrap();

    info!("Setting max sink voltage");
    external::set_max_sink_voltage(controller_context, GlobalPortId(0), Some(5000))
        .await
        .unwrap();

    info!("Clearing dead battery flag");
    external::clear_dead_battery_flag(controller_context, GlobalPortId(0))
        .await
        .unwrap();

    info!("Reconfiguring retimer");
    external::reconfigure_retimer(controller_context, GlobalPortId(0))
        .await
        .unwrap();
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
        controllers,
        power_policy_publisher,
        power_policy_subscriber,
    );

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    if service.register_comms().is_err() {
        error!("Failed to register type-c service endpoint, service already registered?");
        return;
    }

    loop {
        let event = match service.wait_next(controllers).await {
            Ok(event) => event,
            Err(e) => {
                error!("Error waiting for next event: {e:?}");
                continue;
            }
        };

        // Note: must call process_event before so port status is cached for everything else
        if let Err(e) = service.process_event(event, controllers).await {
            error!("Type-C service processing error: {e:#?}");
        }
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let context = CONTEXT.init(embedded_services::type_c::controller::Context::new());

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(service_task(context, controller_list));
        spawner.must_spawn(task(spawner, context));
        spawner.must_spawn(controller_task(context, controller_list));
    });
}
