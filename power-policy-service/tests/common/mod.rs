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
use power_policy_service::psu::EventReceivers;
use power_policy_service::service::Service;

pub mod mock;

use mock::Mock;

use crate::common::mock::FnCall;

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
pub type ServiceType<'device, 'device_storage, 'sender_storage> = Service<
    'device,
    'device_storage,
    'sender_storage,
    DeviceType<'device>,
    DynamicSender<'sender_storage, ServiceEvent<'device, DeviceType<'device>>>,
>;

async fn power_policy_task<'device, 'device_storage, 'sender_storage, const N: usize>(
    completion_signal: &'device Signal<GlobalRawMutex, ()>,
    mut power_policy: ServiceType<'device, 'device_storage, 'sender_storage>,
    mut event_receivers: EventReceivers<'device, N, DeviceType<'device>, DynamicReceiver<'device, EventData>>,
) {
    while let Either::First(result) = select(event_receivers.wait_event(), completion_signal.wait()).await {
        power_policy.process_psu_event(result).await.unwrap();
    }
}

/// This trait is a workaround for Rust's current limitations on closures returning a generic future.
///
/// The trait we want to express for `run_test` is something like:
/// ```
/// for<'a> F: FnOnce(
/// &'a Mutex<GlobalRawMutex, Mock<'a, DynamicSender<'a, EventData>>>,
/// &'a Signal<GlobalRawMutex, (usize, FnCall)>,
/// &'a Mutex<GlobalRawMutex, Mock<'a, DynamicSender<'a, EventData>>>,
/// &'a Signal<GlobalRawMutex, (usize, FnCall)>
/// ) -> impl (Future<Output = ()> + 'a)
/// ```
/// However, `impl (Future<Output = ()> + 'a)` is not real syntax. This could be done with the unstable feature type_alias_impl_trait,
/// but we use this helper trait so as to not require use of nightly.
pub trait TestArgsFnOnce<'a, Arg0: 'a, Arg1: 'a, Arg2: 'a, Arg3: 'a, Arg4: 'a>:
    FnOnce(Arg0, Arg1, Arg2, Arg3, Arg4) -> Self::Fut
{
    type Fut: Future<Output = ()>;
}

impl<'a, Arg0: 'a, Arg1: 'a, Arg2: 'a, Arg3: 'a, Arg4: 'a, F, Fut> TestArgsFnOnce<'a, Arg0, Arg1, Arg2, Arg3, Arg4>
    for F
where
    F: FnOnce(Arg0, Arg1, Arg2, Arg3, Arg4) -> Fut,
    Fut: Future<Output = ()>,
{
    type Fut = Fut;
}

pub async fn run_test<F>(timeout: Duration, test: F)
where
    for<'a> F: TestArgsFnOnce<
            'a,
            DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
            &'a DeviceType<'a>,
            &'a Signal<GlobalRawMutex, (usize, FnCall)>,
            &'a DeviceType<'a>,
            &'a Signal<GlobalRawMutex, (usize, FnCall)>,
        >,
{
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

    let service_context = power_policy_service::service::context::Context::new();
    let psu_registration = [&device0, &device1];
    let completion_signal = Signal::new();

    // Ideally F would have two lifetime arguments: 'device and 'sender because the event type requires 'device: 'sender.
    // But Rust doesn't currently support syntax like `for<'device, 'sender> ... where 'device: 'sender`. So we just
    // use a single lifetime. However, the unified lifetime makes the drop-checker think that dropping the channel
    // could be unsafe. We use ManuallyDrop to disable the drop and make the drop-checker happy. None of the types
    // here do any clean-up in their Drop impls so we don't have to worry about any sort of leaks.
    let service_event_channel: ManuallyDrop<
        Channel<GlobalRawMutex, ServiceEvent<'_, DeviceType<'_>>, EVENT_CHANNEL_SIZE>,
    > = ManuallyDrop::new(Channel::new());
    let mut service_sender = [service_event_channel.dyn_sender()];
    let _service_receiver = service_event_channel.dyn_receiver();

    let power_policy = power_policy_service::service::Service::new(
        psu_registration.as_slice(),
        &mut service_sender,
        &service_context,
        Default::default(),
    );

    with_timeout(
        timeout,
        join(
            power_policy_task(
                &completion_signal,
                power_policy,
                EventReceivers::new([&device0, &device1], [device0_receiver, device1_receiver]),
            ),
            async {
                test(_service_receiver, &device0, &device0_signal, &device1, &device1_signal).await;
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
