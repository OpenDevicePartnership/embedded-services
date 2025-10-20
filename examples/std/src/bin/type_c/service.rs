use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
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
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};
use type_c_service::wrapper::message::*;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: power::policy::DeviceId = power::policy::DeviceId(0);
const DELAY_MS: u64 = 1000;

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
    state: &'static mock_controller::ControllerState,
    context: &'static Context,
    controller_list: &'static IntrusiveList,
) {
    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        context,
        CONTROLLER0_ID,
        0, // CFU component ID (unused)
        [(PORT0_ID, POWER0_ID)],
    ));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

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
async fn task(spawner: Spawner, context: &'static Context, controller_list: &'static IntrusiveList) {
    embedded_services::init().await;

    // Register debug accessory listener
    static LISTENER: OnceLock<debug::Listener> = OnceLock::new();
    let listener = LISTENER.get_or_init(debug::Listener::new);
    comms::register_endpoint(listener, &listener.tp).await.unwrap();

    static STATE: OnceLock<mock_controller::ControllerState> = OnceLock::new();
    let state = STATE.get_or_init(mock_controller::ControllerState::new);

    info!("Starting controller task");
    spawner.must_spawn(controller_task(state, context, controller_list));
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

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTEXT.init(embedded_services::type_c::controller::Context::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task(Default::default()));
        spawner.must_spawn(service_task(controller_context, controller_list));
        spawner.must_spawn(task(spawner, controller_context, controller_list));
    });
}
