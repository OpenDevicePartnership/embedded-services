use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embedded_services::power::policy::policy;
use embedded_services::power::{self};
use embedded_services::type_c::ControllerId;
use embedded_services::type_c::controller::Context;
use embedded_services::{GlobalRawMutex, IntrusiveList, comms};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::type_c::Current;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use std_examples::type_c::mock_controller::Wrapper;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};
use type_c_service::wrapper::message::*;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power::policy::DeviceId = power::policy::DeviceId(0);
const DELAY_MS: u64 = 1000;

const POLICY_CHANNEL_SIZE: usize = 1;

mod debug {
    use embedded_services::{
        comms::{self, Endpoint, EndpointID, Internal},
        info,
        type_c::comms::DebugAccessoryMessage,
    };

    pub struct Listener {
        pub tp: Endpoint,
    }

    impl Listener {
        pub fn new() -> Self {
            Self {
                tp: Endpoint::uninit(EndpointID::Internal(Internal::Usbc)),
            }
        }
    }

    impl comms::MailboxDelegate for Listener {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            if let Some(message) = message.data.get::<DebugAccessoryMessage>() {
                if message.connected {
                    info!("Port{}: Debug accessory connected", message.port.0);
                } else {
                    info!("Port{}: Debug accessory disconnected", message.port.0);
                }
            }

            Ok(())
        }
    }
}

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
async fn task(
    spawner: Spawner,
    wrapper: &'static Wrapper<'static>,
    controller: &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
    state: &'static mock_controller::ControllerState,
) {
    embedded_services::init().await;

    // Register debug accessory listener
    static LISTENER: OnceLock<debug::Listener> = OnceLock::new();
    let listener = LISTENER.get_or_init(debug::Listener::new);
    comms::register_endpoint(listener, &listener.tp).await.unwrap();

    info!("Starting controller task");
    spawner.must_spawn(controller_task(wrapper, controller));
    // Wait for controller to be registered
    Timer::after_secs(1).await;

    info!("Simulating connection");
    state.connect_sink(Current::UsbDefault.into(), false).await;
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
async fn power_policy_service_task(policy: &'static power_policy_service::PowerPolicy<POLICY_CHANNEL_SIZE>) {
    power_policy_service::task::task(
        policy,
        None::<[&std_examples::type_c::DummyPowerDevice<POLICY_CHANNEL_SIZE>; 0]>,
        None::<[&std_examples::type_c::DummyCharger; 0]>,
    )
    .await
    .expect("Failed to start power policy service task");
}

#[embassy_executor::task]
async fn service_task(
    controller_context: &'static Context,
    controllers: &'static IntrusiveList,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static policy::Context<POLICY_CHANNEL_SIZE>,
) {
    info!("Starting type-c task");

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

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

    type_c_service::task::task(service, wrappers, power_policy_context, cfu_client).await;
}

fn create_wrapper(
    context: &'static Context,
    power_policy_context: &'static policy::Context<POLICY_CHANNEL_SIZE>,
) -> (
    &'static mut Wrapper<'static>,
    &'static Mutex<GlobalRawMutex, mock_controller::Controller<'static>>,
    &'static mock_controller::ControllerState,
) {
    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static STORAGE: StaticCell<Storage<1, GlobalRawMutex, POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [(PORT0_ID, POWER0_ID)],
        power_policy_context,
    ));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex, POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let referenced = REFERENCED.init(
        storage
            .create_referenced()
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

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTEXT.init(embedded_services::type_c::controller::Context::new());

    static POWER_POLICY_SERVICE: StaticCell<power_policy_service::PowerPolicy<POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let power_policy_service = POWER_POLICY_SERVICE.init(power_policy_service::PowerPolicy::new(
        power_policy_service::Config::default(),
    ));

    let (wrapper, controller, state) = create_wrapper(controller_context, &power_policy_service.context);

    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service_task(power_policy_service));
        spawner.must_spawn(service_task(
            controller_context,
            controller_list,
            [wrapper],
            &power_policy_service.context,
        ));
        spawner.must_spawn(task(spawner, wrapper, controller, state));
    });
}
