use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::power::{self};
use embedded_services::transformers::object::Object;
use embedded_services::type_c::{ControllerId, controller};
use embedded_services::{GlobalRawMutex, comms};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::type_c::Current;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};
use type_c_service::wrapper::message::*;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: GlobalPortId = GlobalPortId(0);
const POWER0: power::policy::DeviceId = power::policy::DeviceId(0);
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
                    info!("{}: Debug accessory connected", message.port);
                } else {
                    info!("{}: Debug accessory disconnected", message.port);
                }
            }

            Ok(())
        }
    }
}

#[embassy_executor::task]
async fn controller_task(state: &'static mock_controller::ControllerState) {
    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        CONTROLLER0,
        0, // CFU component ID (unused)
        [(PORT0, POWER0)],
    ));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

    static WRAPPER: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let controller = mock_controller::Controller::new(state);
    let wrapper = WRAPPER.init(
        mock_controller::Wrapper::try_new(controller, referenced, crate::mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    wrapper.register().await.unwrap();

    wrapper.get_inner().await.custom_function();

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
            info!("{}: PD alert received: {:?}", port, ado);
        }

        if let Err(e) = wrapper.finalize(output).await {
            error!("Error finalizing output: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    controller::init();

    // Register debug accessory listener
    static LISTENER: OnceLock<debug::Listener> = OnceLock::new();
    let listener = LISTENER.get_or_init(debug::Listener::new);
    comms::register_endpoint(listener, &listener.tp).await.unwrap();

    static STATE: OnceLock<mock_controller::ControllerState> = OnceLock::new();
    let state = STATE.get_or_init(mock_controller::ControllerState::new);

    info!("Starting controller task");
    spawner.must_spawn(controller_task(state));
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
