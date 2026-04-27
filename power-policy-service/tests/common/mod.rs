#![allow(clippy::unwrap_used)]
#![allow(dead_code)]
#![allow(clippy::panic)]
use std::mem::ManuallyDrop;

use embassy_futures::{
    join::join,
    select::{Either, select},
};
use embassy_sync::{
    channel::{Channel, DynamicReceiver, DynamicSender},
    mutex::Mutex,
    once_lock::OnceLock,
    signal::Signal,
};
use embassy_time::{Duration, with_timeout};
use embedded_services::GlobalRawMutex;
use power_policy_interface::psu::event::EventData;
use power_policy_interface::{
    capability::{ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    service::{UnconstrainedState, event::Event as ServiceEvent},
};
use power_policy_service::service::{Service, config::Config};
use power_policy_service::{psu::PsuEventReceivers, service::registration::ArrayRegistration};

pub mod mock;

use mock::Mock;

use crate::common::mock::{ChargerType, FnCall};

pub const MINIMAL_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 500,
};

pub const LOW_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

#[allow(dead_code)]
pub const HIGH_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

const EVENT_CHANNEL_SIZE: usize = 4;

pub type DeviceType<'a> = Mutex<GlobalRawMutex, Mock<'a, DynamicSender<'a, EventData>>>;
pub type ServiceType<'device, 'sender> = Service<
    'device,
    ArrayRegistration<
        'device,
        DeviceType<'device>,
        2,
        DynamicSender<'sender, ServiceEvent<'device, DeviceType<'device>>>,
        1,
        ChargerType<'device>,
        0,
    >,
>;

pub type ServiceMutex<'device, 'sender> = Mutex<GlobalRawMutex, ServiceType<'device, 'sender>>;

async fn power_policy_task<'device, 'sender, const N: usize>(
    completion_signal: &'device Signal<GlobalRawMutex, ()>,
    power_policy: &ServiceMutex<'device, 'sender>,
    mut event_receivers: PsuEventReceivers<'device, N, DeviceType<'device>, DynamicReceiver<'device, EventData>>,
) {
    while let Either::First(result) = select(event_receivers.wait_event(), completion_signal.wait()).await {
        power_policy.lock().await.process_psu_event(result).await.unwrap();
    }
}

/// Trait for runnable tests.
///
/// This exists because there are lifetime issues with being generic over FnOnce or FnMut.
/// Those can be resolved, but having a dedicated trait is simpler.
pub trait Test {
    fn run<'a>(
        &mut self,
        service: &'a ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &'a DeviceType<'a>,
        device0_signal: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &'a DeviceType<'a>,
        device1_signal: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    ) -> impl Future<Output = ()>;
}

pub async fn run_test(timeout: Duration, mut test: impl Test, config: Config) {
    // Tokio runs tests in parallel, but logging is global so we need to run tests sequentially to avoid interleaved logs.
    static TEST_MUTEX: OnceLock<Mutex<GlobalRawMutex, ()>> = OnceLock::new();
    let test_mutex = TEST_MUTEX.get_or_init(|| Mutex::new(()));
    let _lock = test_mutex.lock().await;

    // Initialize logging, ignore the error if the logger was already initialized by another test.
    let _ = env_logger::builder().filter_level(log::LevelFilter::Info).try_init();
    embedded_services::init().await;

    let device0_signal = Signal::new();
    let device0_event_channel: Channel<GlobalRawMutex, EventData, EVENT_CHANNEL_SIZE> = Channel::new();
    let device0_sender = device0_event_channel.dyn_sender();
    let device0_receiver = device0_event_channel.dyn_receiver();
    let device0 = Mutex::new(Mock::new("PSU0", device0_sender, &device0_signal));

    let device1_signal = Signal::new();
    let device1_event_channel: Channel<GlobalRawMutex, EventData, EVENT_CHANNEL_SIZE> = Channel::new();
    let device1_sender = device1_event_channel.dyn_sender();
    let device1_receiver = device1_event_channel.dyn_receiver();
    let device1 = Mutex::new(Mock::new("PSU1", device1_sender, &device1_signal));

    let completion_signal = Signal::new();

    // For simplicity, Test::run is only generic over a single lifetime. But this causes issues with the drop checker because
    // the device lifetime doesn't outlive the channel lifetime from its perspective. Use ManuallyDrop to work around this.
    let service_event_channel: ManuallyDrop<
        Channel<GlobalRawMutex, ServiceEvent<'_, DeviceType<'_>>, EVENT_CHANNEL_SIZE>,
    > = ManuallyDrop::new(Channel::new());
    let service_receiver = service_event_channel.dyn_receiver();

    let power_policy_registration = ArrayRegistration {
        psus: [&device0, &device1],
        service_senders: [service_event_channel.dyn_sender()],
        chargers: [],
    };

    let power_policy = Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        config,
    ));

    with_timeout(
        timeout,
        join(
            power_policy_task(
                &completion_signal,
                &power_policy,
                PsuEventReceivers::new([&device0, &device1], [device0_receiver, device1_receiver]),
            ),
            async {
                test.run(
                    &power_policy,
                    service_receiver,
                    &device0,
                    &device0_signal,
                    &device1,
                    &device1_signal,
                )
                .await;
                completion_signal.signal(());
            },
        ),
    )
    .await
    .unwrap();
}

pub async fn assert_consumer_disconnected<'a>(
    receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    expected_device: &DeviceType<'a>,
) {
    let ServiceEvent::ConsumerDisconnected(device) = receiver.receive().await else {
        panic!("Expected ConsumerDisconnected event");
    };
    assert_eq!(device as *const _, expected_device as *const _);
}

pub async fn assert_consumer_connected<'a>(
    receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    expected_device: &DeviceType<'a>,
    expected_capability: ConsumerPowerCapability,
) {
    let ServiceEvent::ConsumerConnected(device, capability) = receiver.receive().await else {
        panic!("Expected ConsumerConnected event");
    };
    assert_eq!(device as *const _, expected_device as *const _);
    assert_eq!(capability, expected_capability);
}

pub async fn assert_provider_disconnected<'a>(
    receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    expected_device: &DeviceType<'a>,
) {
    let ServiceEvent::ProviderDisconnected(device) = receiver.receive().await else {
        panic!("Expected ProviderDisconnected event");
    };
    assert_eq!(device as *const _, expected_device as *const _);
}

pub async fn assert_provider_connected<'a>(
    receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    expected_device: &DeviceType<'a>,
    expected_capability: ProviderPowerCapability,
) {
    let ServiceEvent::ProviderConnected(device, capability) = receiver.receive().await else {
        panic!("Expected ProviderConnected event");
    };
    assert_eq!(device as *const _, expected_device as *const _);
    assert_eq!(capability, expected_capability);
}

pub async fn assert_unconstrained<'a>(
    receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    expected_state: UnconstrainedState,
) {
    let ServiceEvent::Unconstrained(state) = receiver.receive().await else {
        panic!("Expected Unconstrained event");
    };
    assert_eq!(state, expected_state);
}

pub fn assert_no_event<'a>(receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>) {
    assert!(receiver.try_receive().is_err());
}
