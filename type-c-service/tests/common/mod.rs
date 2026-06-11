#![allow(dead_code)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use embassy_futures::{
    join::{join, join3},
    select::{Either, select},
};
use embassy_sync::{
    mutex::Mutex,
    once_lock::OnceLock,
    pubsub::{DynSubscriber, PubSubChannel},
    watch::{DynReceiver, Watch},
};
use embassy_time::{Duration, with_timeout};
use embedded_services::{
    GlobalRawMutex, broadcaster, info,
    power::{self, policy},
    type_c::{self, ControllerId},
};
use embedded_usb_pd::GlobalPortId;
use paste::paste;
use static_cell::StaticCell;

pub mod mock;
pub const DEFAULT_TEST_DURATION: Duration = Duration::from_secs(15);

pub const DEFAULT_PER_CALL_TIMEOUT: Duration = Duration::from_secs(1);

pub const CONTROLLER0_ID: ControllerId = ControllerId(0);
pub const PORT0_ID: GlobalPortId = GlobalPortId(0);
pub const POWER0_ID: power::policy::DeviceId = power::policy::DeviceId(0);
pub const CFU0_ID: u8 = 0x00;

pub const CONTROLLER1_ID: ControllerId = ControllerId(1);
pub const PORT1_ID: GlobalPortId = GlobalPortId(1);
pub const POWER1_ID: power::policy::DeviceId = power::policy::DeviceId(1);
pub const CFU1_ID: u8 = 0x01;

pub const CONTROLLER2_ID: ControllerId = ControllerId(2);
pub const PORT2_ID: GlobalPortId = GlobalPortId(2);
pub const POWER2_ID: power::policy::DeviceId = power::policy::DeviceId(2);
pub const CFU2_ID: u8 = 0x02;

/// Integration test trait
///
/// Directly taking async closures is messy and requires an intermediate trait anyway
pub trait Test {
    /// Run the test
    fn run(
        &mut self,
        type_c_receiver: DynSubscriber<'static, type_c::comms::CommsMessage>,
        power_policy_event_receiver: DynSubscriber<'static, policy::CommsMessage>,
        port0: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port1: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        port2: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    ) -> impl Future<Output = ()>;
}

async fn controller_task(wrapper: &'static mock::Wrapper<'static>, mut completion_signal: DynReceiver<'static, ()>) {
    while let Either::First(result) = select(wrapper.process_next_event(), completion_signal.get()).await {
        result.unwrap();
    }
}

async fn power_policy_task(
    config: power_policy_service::config::Config,
    mut completion_signal: DynReceiver<'static, ()>,
) {
    if let Either::First(result) = select(power_policy_service::task::task(config), completion_signal.get()).await {
        panic!("Power policy task completed before test end: {result:?}");
    }
}

async fn type_c_service_task(
    config: type_c_service::service::config::Config,
    mut completion_signal: DynReceiver<'static, ()>,
) {
    if let Either::First(result) = select(type_c_service::task::task(config), completion_signal.get()).await {
        panic!("Type-C service task completed before test end: {result:?}");
    }
}

pub struct PortComponents<'a> {
    pub state: &'a Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    pub wrapper: &'a mock::Wrapper<'static>,
}

macro_rules! define_controller {
    ($name:ident, $controller_id:expr, $port_id:expr, $power_id:expr, $cfu_id:expr) => {
        paste! {
            static [<$name _STORAGE>]: ::static_cell::StaticCell<::type_c_service::wrapper::backing::Storage<1, GlobalRawMutex>> = ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _storage>] =
                [<$name _STORAGE>].init(::type_c_service::wrapper::backing::Storage::new($controller_id, $cfu_id, [($port_id, $power_id)]));

            static [<$name _REFERENCED>]: ::static_cell::StaticCell<::type_c_service::wrapper::backing::ReferencedStorage<1, GlobalRawMutex>> =
                ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _referenced>] = [<$name _REFERENCED>].init(
                [<$name _storage>]
                    .create_referenced()
                    .expect("Failed to create referenced storage"),
            );

            static [<$name _INTERRUPT>]: ::static_cell::StaticCell<::embassy_sync::signal::Signal<::embedded_services::GlobalRawMutex, ()>> = ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _interrupt>] = [<$name _INTERRUPT>].init(::embassy_sync::signal::Signal::new());

            static [<$name _STATE>]: ::static_cell::StaticCell<::embassy_sync::mutex::Mutex<::embedded_services::GlobalRawMutex, mock::ControllerState<'static>>> =
                ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _state>] = [<$name _STATE>]
                .init(::embassy_sync::mutex::Mutex::new(mock::ControllerState::new([<$name _interrupt>])));

            static [<$name _DEVICE>]: ::static_cell::StaticCell<::embassy_sync::mutex::Mutex<::embedded_services::GlobalRawMutex, mock::Controller<'static>>> = ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _device>] = [<$name _DEVICE>]
                .init(::embassy_sync::mutex::Mutex::new(mock::Controller::new([<$name _state>], [<$name _interrupt>])));

            static [<$name _WRAPPER>]: ::static_cell::StaticCell<mock::Wrapper<'static>> = ::static_cell::StaticCell::new();
            #[allow(non_snake_case)]
            let [<$name _wrapper>] = [<$name _WRAPPER>].init(
                mock::Wrapper::try_new(
                    [<$name _device>],
                    Default::default(),
                    [<$name _referenced>],
                    mock::Validator,
                )
                .expect("Failed to create wrapper")
            );
            #[allow(non_snake_case)]
            let $name = PortComponents {
                state: [<$name _state>],
                wrapper: [<$name _wrapper>],
            };
        }
    };
}

/// Initialize services and run an integration test
pub async fn run_test(
    duration: Duration,
    type_c_service_config: type_c_service::service::config::Config,
    power_policy_service_config: power_policy_service::config::Config,
    mut test: impl Test,
) {
    // Tokio runs tests in parallel, but logging is global so we need to run tests sequentially to avoid interleaved logs.
    static TEST_MUTEX: OnceLock<Mutex<GlobalRawMutex, ()>> = OnceLock::new();
    let test_mutex = TEST_MUTEX.get_or_init(|| Mutex::new(()));
    let _lock = test_mutex.lock().await;

    // Initialize logging, ignore the error if the logger was already initialized by another test.
    let _ = env_logger::builder().filter_level(log::LevelFilter::Info).try_init();

    // 5 for the three controller tasks, power policy task, and type-C service task.
    static COMPLETION_SIGNAL: StaticCell<Watch<GlobalRawMutex, (), 5>> = StaticCell::new();
    let completion_signal = COMPLETION_SIGNAL.init(Watch::new());

    embedded_services::init().await;

    info!("Creating power policy service event channel");
    static POWER_POLICY_SERVICE_EVENT_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, policy::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();
    let power_policy_service_event_channel = POWER_POLICY_SERVICE_EVENT_CHANNEL.init(PubSubChannel::new());

    let power_policy_service_event_publisher = power_policy_service_event_channel.dyn_immediate_publisher();
    let power_policy_service_event_subscriber = power_policy_service_event_channel.dyn_subscriber().unwrap();

    static POWER_POLICY_SERVICE_EVENT_RECEIVER: StaticCell<
        broadcaster::immediate::Receiver<'static, policy::CommsMessage>,
    > = StaticCell::new();
    let power_policy_service_event_receiver = POWER_POLICY_SERVICE_EVENT_RECEIVER.init(
        broadcaster::immediate::Receiver::new(power_policy_service_event_publisher),
    );

    policy::policy::register_message_receiver(power_policy_service_event_receiver).unwrap();

    info!("Creating type-C service event channel");
    static TYPE_C_SERVICE_EVENT_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, type_c::comms::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();
    let type_c_service_event_channel = TYPE_C_SERVICE_EVENT_CHANNEL.init(PubSubChannel::new());

    let type_c_service_event_publisher = type_c_service_event_channel.dyn_immediate_publisher();
    let type_c_service_event_subscriber = type_c_service_event_channel.dyn_subscriber().unwrap();

    static TYPE_C_SERVICE_EVENT_RECEIVER: StaticCell<
        broadcaster::immediate::Receiver<'static, type_c::comms::CommsMessage>,
    > = StaticCell::new();
    let type_c_service_event_receiver =
        TYPE_C_SERVICE_EVENT_RECEIVER.init(broadcaster::immediate::Receiver::new(type_c_service_event_publisher));

    type_c::controller::register_message_receiver(type_c_service_event_receiver).unwrap();

    define_controller!(CONTROLLER0, CONTROLLER0_ID, PORT0_ID, POWER0_ID, CFU0_ID);
    let PortComponents {
        state: controller0_state,
        wrapper: controller0_wrapper,
    } = CONTROLLER0;
    controller0_wrapper.register().await.unwrap();

    define_controller!(CONTROLLER1, CONTROLLER1_ID, PORT1_ID, POWER1_ID, CFU1_ID);
    let PortComponents {
        state: controller1_state,
        wrapper: controller1_wrapper,
    } = CONTROLLER1;
    controller1_wrapper.register().await.unwrap();

    define_controller!(CONTROLLER2, CONTROLLER2_ID, PORT2_ID, POWER2_ID, CFU2_ID);
    let PortComponents {
        state: controller2_state,
        wrapper: controller2_wrapper,
    } = CONTROLLER2;
    controller2_wrapper.register().await.unwrap();

    with_timeout(
        duration,
        join3(
            join(
                power_policy_task(power_policy_service_config, completion_signal.dyn_receiver().unwrap()),
                type_c_service_task(type_c_service_config, completion_signal.dyn_receiver().unwrap()),
            ),
            join3(
                controller_task(controller0_wrapper, completion_signal.dyn_receiver().unwrap()),
                controller_task(controller1_wrapper, completion_signal.dyn_receiver().unwrap()),
                controller_task(controller2_wrapper, completion_signal.dyn_receiver().unwrap()),
            ),
            async {
                test.run(
                    type_c_service_event_subscriber,
                    power_policy_service_event_subscriber,
                    controller0_state,
                    controller1_state,
                    controller2_state,
                )
                .await;
                completion_signal.sender().send(());
            },
        ),
    )
    .await
    .unwrap();
}
