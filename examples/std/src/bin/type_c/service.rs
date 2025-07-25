use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::comms;
use embedded_services::power::{self, policy};
use embedded_services::transformers::object::Object;
use embedded_services::type_c::{ControllerId, controller};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::type_c::Current;
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;

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
async fn controller_task(wrapper: &'static mock_controller::Wrapper<'static>) {
    wrapper.register().await.unwrap();

    wrapper.get_inner().await.custom_function();

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

    // Register debug accessory listener
    static LISTENER: OnceLock<debug::Listener> = OnceLock::new();
    let listener = LISTENER.get_or_init(debug::Listener::new);
    comms::register_endpoint(listener, &listener.tp).await.unwrap();

    static STATE: OnceLock<mock_controller::ControllerState> = OnceLock::new();
    let state = STATE.get_or_init(mock_controller::ControllerState::new);

    let controller = mock_controller::Controller::new(state);
    static WRAPPER: OnceLock<mock_controller::Wrapper> = OnceLock::new();
    let wrapper = WRAPPER.get_or_init(|| {
        mock_controller::Wrapper::new(
            embedded_services::type_c::controller::Device::new(CONTROLLER0, &[PORT0]),
            [policy::device::Device::new(POWER0)],
            embedded_services::cfu::component::CfuDevice::new(0x00),
            controller,
            crate::mock_controller::Validator,
        )
    });

    info!("Starting controller task");
    spawner.must_spawn(controller_task(&wrapper));
    // Wait for controller to be registered
    Timer::after_secs(1).await;

    info!("Simulating connection");
    state.connect_sink(Current::UsbDefault.into(), false).await;
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
        spawner.must_spawn(type_c_service::task());
        spawner.must_spawn(task(spawner));
    });
}
