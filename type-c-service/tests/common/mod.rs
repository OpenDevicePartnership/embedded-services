use std::mem::ManuallyDrop;

use embassy_futures::{
    join::{join, join3},
    select::{Either, select},
};
use embassy_sync::{
    channel::{Channel, DynamicReceiver, DynamicSender},
    mutex::Mutex,
    once_lock::OnceLock,
    watch,
};
use embassy_time::{Duration, with_timeout};
use embedded_services::{GlobalRawMutex, event::Sender};
use embedded_usb_pd::LocalPortId;
use paste::paste;
use power_policy_interface::charger::mock::NoopCharger;
use type_c_service::service::registration::PortData;

pub const DEFAULT_TEST_DURATION: Duration = Duration::from_secs(5);

pub const DEFAULT_PER_CALL_TIMEOUT: Duration = Duration::from_secs(1);

/// Total number of type-C ports
pub const TYPE_C_PORT_COUNT: usize = 3;
/// Number of senders for type-c service events
pub const TYPE_C_SERVICE_SENDER_COUNT: usize = 1;
/// Number of senders for power policy events
pub const POWER_POLICY_SENDER_COUNT: usize = 1;

/// Mutex wrapped controller mock
pub type ControllerMockMutexType = Mutex<GlobalRawMutex, type_c_interface_mocks::controller::Mock>;

/// [`type_c_service::controller::Port`] sender to type-C service
pub type PortTypeCSender<'a> = DynamicSender<'a, type_c_interface::service::event::PortEventData>;
/// Corresponding receiver for [`PortTypeCSender`]
pub type PortTypeCReceiver<'a> = DynamicReceiver<'a, type_c_interface::service::event::PortEventData>;
/// [`type_c_service::controller::Port`] sender to power policy service
pub type PortPowerSender<'a> = DynamicSender<'a, power_policy_interface::psu::event::EventData>;
/// Corresponding receiver for [`PortPowerSender`]
pub type PortPowerReceiver<'a> = DynamicReceiver<'a, power_policy_interface::psu::event::EventData>;
/// [`type_c_service::controller::Port`] sender for loopback events
pub type PortLoopbackSender<'a> = DynamicSender<'a, type_c_service::controller::event::Loopback>;
/// Shared port state type
pub type PortSharedState = Mutex<GlobalRawMutex, type_c_service::controller::state::SharedState>;
/// Port type
pub type PortMutexType<'port, 'ch> = Mutex<
    GlobalRawMutex,
    type_c_service::controller::Port<
        'port,
        // Underlying controller
        ControllerMockMutexType,
        // Shared state between the event receiver and port logic
        PortSharedState,
        // Sender to the type-C service
        PortTypeCSender<'ch>,
        // Sender to the power policy
        PortPowerSender<'ch>,
        // Loopback sender
        PortLoopbackSender<'ch>,
    >,
>;

/// Sender for events broadcast by the power policy service
pub type PowerPolicyServiceSender<'port, 'ch> = PowerPolicyServiceEventRouter<'port, 'ch>;
/// Receiver for events broadcast by the power policy service
pub type PowerPolicyServiceReceiver<'port, 'ch> =
    DynamicReceiver<'ch, power_policy_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>>;
/// Power policy registration type
pub type PowerPolicyRegistrationType<'port, 'ch> = power_policy_service::service::registration::ArrayRegistration<
    'port,
    // PSU type
    PortMutexType<'port, 'ch>,
    // PSU count
    TYPE_C_PORT_COUNT,
    // Senders for events broadcast by the service
    PowerPolicyServiceSender<'port, 'ch>,
    // Number of registered service event senders
    POWER_POLICY_SENDER_COUNT,
    // Charger type
    Mutex<GlobalRawMutex, NoopCharger>,
    // Charger count
    0,
>;
/// Power policy service type
pub type PowerPolicyServiceMutexType<'port, 'ch> =
    Mutex<GlobalRawMutex, power_policy_service::service::Service<'port, PowerPolicyRegistrationType<'port, 'ch>>>;

/// Sender for events broadcast by the type-C service
pub type TypeCServiceSender<'port, 'ch> =
    DynamicSender<'ch, type_c_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>>;
/// Receiver for events broadcast by the type-C service
pub type TypeCServiceReceiver<'port, 'ch> =
    DynamicReceiver<'ch, type_c_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>>;
/// Type-C service registration type
pub type TypeCRegistrationType<'port, 'ch> = type_c_service::service::registration::ArrayRegistration<
    'port,
    // Port type
    PortMutexType<'port, 'ch>,
    // Number of type-C ports
    TYPE_C_PORT_COUNT,
    // Senders for events broadcast by the service
    TypeCServiceSender<'port, 'ch>,
    // Number of registered service event senders
    TYPE_C_SERVICE_SENDER_COUNT,
>;
/// Type-C service type
pub type TypeCServiceMutexType<'port, 'ch> =
    Mutex<GlobalRawMutex, type_c_service::service::Service<'port, TypeCRegistrationType<'port, 'ch>>>;

/// Default channel size to use
pub const CHANNEL_SIZE: usize = 4;

/// Struct to pass port components to a test implementation.
pub struct TestPort<'port, 'ch> {
    /// Port logic
    pub port: &'port PortMutexType<'port, 'ch>,
    /// Underlying controller mock
    pub mock: &'port ControllerMockMutexType,
}

/// Integration test trait
///
/// Directly taking async closures is messy and requires an intermediate trait anyway
pub trait Test {
    /// Run the test
    fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        port1: TestPort<'port, 'ch>,
        port2: TestPort<'port, 'ch>,
    ) -> impl Future<Output = ()>;
}

/// Used by the [`define_port`] macro to work around macro hygiene issues.
struct PortComponents<'port, 'ch> {
    port: PortMutexType<'port, 'ch>,
    mock: &'port ControllerMockMutexType,
    type_c_receiver: PortTypeCReceiver<'ch>,
    power_policy_receiver: PortPowerReceiver<'ch>,
}

macro_rules! define_port {
    ($name:ident, $mock_name:expr, $port_name:expr, $config:expr, $local_id:expr) => {
        paste! { let [<$name _type_c_channel>]: Channel<
            GlobalRawMutex,
            type_c_interface::service::event::PortEventData,
            CHANNEL_SIZE,
        > = Channel::new(); }
        paste! { let [<$name _type_c_sender>] = [<$name _type_c_channel>].dyn_sender(); }
        paste! { let [<$name _type_c_receiver>] = [<$name _type_c_channel>].dyn_receiver(); }

        paste! { let [<$name _power_policy_channel>]: Channel<
            GlobalRawMutex,
            power_policy_interface::psu::event::EventData,
            CHANNEL_SIZE,
        > = Channel::new(); }
        paste! { let [<$name _power_policy_sender>] = [<$name _power_policy_channel>].dyn_sender(); }
        paste! { let [<$name _power_policy_receiver>] = [<$name _power_policy_channel>].dyn_receiver(); }

        paste! { let [<$name _loopback_channel>]: Channel<
            GlobalRawMutex,
            type_c_service::controller::event::Loopback,
            CHANNEL_SIZE,
        > = Channel::new(); }
        paste! { let [<$name _loopback_sender>] = [<$name _loopback_channel>].dyn_sender(); }

        paste! { let [<$name _mock>] = Mutex::new(type_c_interface_mocks::controller::Mock::new($mock_name)); }
        paste! { let [<$name _shared_state>] =
        PortSharedState::new(type_c_service::controller::state::SharedState::new()); }
        paste! { let $name = PortComponents {
                port: Mutex::new(type_c_service::controller::Port::new(
                    $port_name,
                    $config,
                    $local_id,
                    &paste! { [<$name _mock>] },
                    &paste! { [<$name _shared_state>] },
                    paste! { [<$name _type_c_sender>] },
                    paste! { [<$name _power_policy_sender>] },
                    paste! { [<$name _loopback_sender>] },
            )),
            mock: &paste! { [<$name _mock>] },
            type_c_receiver: paste! { [<$name _type_c_receiver>] },
            power_policy_receiver: paste! { [<$name _power_policy_receiver>] },
        };
        }
    };
}

/// Router for events from the power policy service. Forwards events to the test receiver and the type-C service.
// TODO: remove this once enum_dispatch is implemented
pub struct PowerPolicyServiceEventRouter<'port, 'ch> {
    /// Sender to the test receiver
    test_sender: DynamicSender<'ch, power_policy_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>>,
    /// Sender to the type-C service
    type_c_sender: DynamicSender<'ch, power_policy_interface::service::event::EventData>,
}

impl<'port, 'ch> Sender<power_policy_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>>
    for PowerPolicyServiceEventRouter<'port, 'ch>
{
    fn try_send(
        &mut self,
        event: power_policy_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>,
    ) -> Option<()> {
        self.test_sender.try_send(event).ok()?;
        self.type_c_sender.try_send(event.into()).ok()
    }

    async fn send(&mut self, event: power_policy_interface::service::event::Event<'port, PortMutexType<'port, 'ch>>) {
        join(self.test_sender.send(event), self.type_c_sender.send(event.into())).await;
    }
}

/// Power policy event loop task
async fn power_policy_task<'psu, 'ch, 'service, 'completion>(
    mut completion_signal: watch::DynReceiver<'completion, ()>,
    power_policy: &'service PowerPolicyServiceMutexType<'psu, 'ch>,
    mut event_receivers: power_policy_service::psu::PsuEventReceivers<
        'psu,
        TYPE_C_PORT_COUNT,
        PortMutexType<'psu, 'ch>,
        DynamicReceiver<'ch, power_policy_interface::psu::event::EventData>,
    >,
) {
    while let Either::First(event) = select(event_receivers.wait_event(), completion_signal.get()).await {
        power_policy.lock().await.process_psu_event(event).await.unwrap();
    }
}

/// Type-C service event loop task
async fn type_c_service_task<'port, 'ch, 'service, 'completion>(
    mut completion_signal: watch::DynReceiver<'completion, ()>,
    service: &'service TypeCServiceMutexType<'port, 'ch>,
    mut event_receiver: type_c_service::service::event_receiver::ArrayEventReceiver<
        'port,
        TYPE_C_PORT_COUNT,
        PortMutexType<'port, 'ch>,
        DynamicReceiver<'ch, type_c_interface::service::event::PortEventData>,
        DynamicReceiver<'ch, power_policy_interface::service::event::EventData>,
    >,
) {
    while let Either::First(event) = select(event_receiver.wait_next(), completion_signal.get()).await {
        service.lock().await.process_event(event).await.unwrap();
    }
}

/// Initialize services and run an integration test
pub async fn run_test(
    duration: Duration,
    type_c_service_config: type_c_service::service::config::Config,
    port_config: [type_c_service::controller::config::Config; TYPE_C_PORT_COUNT],
    mut test: impl Test,
) {
    // Tokio runs tests in parallel, but logging is global so we need to run tests sequentially to avoid interleaved logs.
    static TEST_MUTEX: OnceLock<Mutex<GlobalRawMutex, ()>> = OnceLock::new();
    let test_mutex = TEST_MUTEX.get_or_init(|| Mutex::new(()));
    let _lock = test_mutex.lock().await;

    // Initialize logging, ignore the error if the logger was already initialized by another test.
    let _ = env_logger::builder().filter_level(log::LevelFilter::Info).try_init();
    embedded_services::init().await;

    define_port!(port0, "mock0", "port0", port_config[0], LocalPortId(0));
    let PortComponents {
        port: port0,
        type_c_receiver: port0_type_c_receiver,
        power_policy_receiver: port0_power_policy_receiver,
        mock: port0_mock,
    } = port0;

    define_port!(port1, "mock1", "port1", port_config[1], LocalPortId(0));
    let PortComponents {
        port: port1,
        type_c_receiver: port1_type_c_receiver,
        power_policy_receiver: port1_power_policy_receiver,
        mock: port1_mock,
    } = port1;

    define_port!(port2, "mock2", "port2", port_config[2], LocalPortId(0));
    let PortComponents {
        port: port2,
        type_c_receiver: port2_type_c_receiver,
        power_policy_receiver: port2_power_policy_receiver,
        mock: port2_mock,
    } = port2;

    // Channel to broadcast events from the type-C service
    let type_c_service_channel: ManuallyDrop<
        Channel<GlobalRawMutex, type_c_interface::service::event::Event<'_, PortMutexType<'_, '_>>, CHANNEL_SIZE>,
    > = ManuallyDrop::new(Channel::new());
    let type_c_service_sender = type_c_service_channel.dyn_sender();
    let type_c_service_receiver = type_c_service_channel.dyn_receiver();

    let type_c_service = Mutex::new(type_c_service::service::Service::new(
        type_c_service_config,
        TypeCRegistrationType {
            ports: [&port0, &port1, &port2],
            port_data: [
                PortData {
                    local_port: Some(LocalPortId(0)),
                },
                PortData {
                    local_port: Some(LocalPortId(0)),
                },
                PortData {
                    local_port: Some(LocalPortId(0)),
                },
            ],
            service_senders: [type_c_service_sender],
        },
    ));

    // Channel for events from the power policy service to the type-C service
    let type_c_power_policy_events: ManuallyDrop<
        Channel<GlobalRawMutex, power_policy_interface::service::event::EventData, CHANNEL_SIZE>,
    > = ManuallyDrop::new(Channel::new());
    let type_c_power_policy_sender = type_c_power_policy_events.dyn_sender();
    let type_c_power_policy_receiver = type_c_power_policy_events.dyn_receiver();

    let type_c_service_event_receivers = type_c_service::service::event_receiver::ArrayEventReceiver::new(
        [&port0, &port1, &port2],
        [port0_type_c_receiver, port1_type_c_receiver, port2_type_c_receiver],
        type_c_power_policy_receiver,
    );

    // Channel for events from the power policy service to the test
    let power_policy_service_channel: ManuallyDrop<
        Channel<GlobalRawMutex, power_policy_interface::service::event::Event<'_, PortMutexType<'_, '_>>, CHANNEL_SIZE>,
    > = ManuallyDrop::new(Channel::new());
    let power_policy_service_sender = power_policy_service_channel.dyn_sender();
    let power_policy_service_receiver = power_policy_service_channel.dyn_receiver();

    // Router for power policy service events
    let power_policy_service_event_router = PowerPolicyServiceEventRouter {
        test_sender: power_policy_service_sender,
        type_c_sender: type_c_power_policy_sender,
    };

    let power_policy_service = Mutex::new(power_policy_service::service::Service::new(
        power_policy_service::service::registration::ArrayRegistration {
            psus: [&port0, &port1, &port2],
            chargers: [],
            service_senders: [power_policy_service_event_router],
        },
        Default::default(),
    ));

    let power_policy_event_receiver = power_policy_service::psu::PsuEventReceivers {
        psu_devices: [&port0, &port1, &port2],
        receivers: [
            port0_power_policy_receiver,
            port1_power_policy_receiver,
            port2_power_policy_receiver,
        ],
    };

    let completion_signal: watch::Watch<GlobalRawMutex, (), 2> = watch::Watch::new();
    let completion_sender = completion_signal.dyn_sender();

    with_timeout(
        duration,
        join3(
            power_policy_task(
                completion_signal.dyn_receiver().unwrap(),
                &power_policy_service,
                power_policy_event_receiver,
            ),
            type_c_service_task(
                completion_signal.dyn_receiver().unwrap(),
                &type_c_service,
                type_c_service_event_receivers,
            ),
            async {
                test.run(
                    type_c_service_receiver,
                    power_policy_service_receiver,
                    TestPort {
                        port: &port0,
                        mock: port0_mock,
                    },
                    TestPort {
                        port: &port1,
                        mock: port1_mock,
                    },
                    TestPort {
                        port: &port2,
                        mock: port2_mock,
                    },
                )
                .await;
                completion_sender.send(());
            },
        ),
    )
    .await
    .unwrap();
}
