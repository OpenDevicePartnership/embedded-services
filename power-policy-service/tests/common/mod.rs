//! Common test framework code
use embassy_futures::{
    join::join4,
    select::{Either, select},
};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    once_lock::OnceLock,
    pubsub::{DynSubscriber, PubSubChannel, WaitResult},
    watch::{DynReceiver, Watch},
};
use embassy_time::{self as _, Duration, with_timeout};
use embedded_services::{
    GlobalRawMutex,
    broadcaster::immediate::{self as broadcaster},
    info,
    power::policy::{
        self, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability, UnconstrainedState, action,
    },
};
use static_cell::StaticCell;

pub mod mock;

use mock::Mock;

/// Default test timeout
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default timeout per function call
pub const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Device 0 ID constant
pub const DEVICE0_ID: policy::DeviceId = policy::DeviceId(0);

/// Device 1 ID constant
pub const DEVICE1_ID: policy::DeviceId = policy::DeviceId(1);

/// Low power capability
pub const LOW_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

/// High power capability
pub const HIGH_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

async fn device_task(device: &'static Mock, mut completion_signal: DynReceiver<'static, ()>) {
    while let Either::First(result) = select(device.process_request(), completion_signal.get()).await {
        result.unwrap();
    }
}

/// Trait to allow tests to integrate with common test infrastructure.
pub trait Test {
    /// Run the test.
    fn run_test(
        &mut self,
        device0: action::device::Device<'static, action::Detached>,
        device0_mock: &'static Mock,
        device1: action::device::Device<'static, action::Detached>,
        device1_mock: &'static Mock,
        power_policy_event_receiver: DynSubscriber<'static, policy::CommsMessage>,
    ) -> impl Future<Output = ()>;
}

async fn power_policy_task(
    config: power_policy_service::config::Config,
    mut completion_signal: DynReceiver<'static, ()>,
) {
    if let Either::First(result) = select(power_policy_service::task::task(config), completion_signal.get()).await {
        panic!("Power policy task completed before test end: {result:?}");
    }
}

pub async fn run_test(
    mut test: impl Test,
    test_timeout: Duration,
    power_policy_config: power_policy_service::config::Config,
) {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static COMPLETION_SIGNAL: StaticCell<Watch<GlobalRawMutex, (), 3>> = StaticCell::new();
    let completion_signal = COMPLETION_SIGNAL.init(Watch::new());

    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0: OnceLock<Mock> = OnceLock::new();
    let device0_mock = DEVICE0.get_or_init(|| Mock::new(DEVICE0_ID));
    policy::register_device(device0_mock).unwrap();
    let device0 = device0_mock.device.try_device_action().await.unwrap();

    info!("Creating device 1");
    static DEVICE1: OnceLock<Mock> = OnceLock::new();
    let device1_mock = DEVICE1.get_or_init(|| Mock::new(DEVICE1_ID));
    policy::register_device(device1_mock).unwrap();
    let device1 = device1_mock.device.try_device_action().await.unwrap();

    info!("Creating power policy event channel");
    static CHANNEL: StaticCell<PubSubChannel<NoopRawMutex, policy::CommsMessage, 4, 1, 0>> = StaticCell::new();
    let channel = CHANNEL.init(PubSubChannel::new());

    let publisher = channel.dyn_immediate_publisher();
    let subscriber = channel.dyn_subscriber().unwrap();

    static RECEIVER: StaticCell<broadcaster::Receiver<'static, policy::CommsMessage>> = StaticCell::new();
    let receiver = RECEIVER.init(broadcaster::Receiver::new(publisher));

    policy::policy::register_message_receiver(receiver).unwrap();

    with_timeout(
        test_timeout,
        join4(
            power_policy_task(power_policy_config, completion_signal.dyn_receiver().unwrap()),
            device_task(device0_mock, completion_signal.dyn_receiver().unwrap()),
            device_task(device1_mock, completion_signal.dyn_receiver().unwrap()),
            async {
                test.run_test(device0, device0_mock, device1, device1_mock, subscriber)
                    .await;
                completion_signal.dyn_sender().send(());
            },
        ),
    )
    .await
    .expect("Test timeout");
}

pub async fn assert_consumer_disconnected(
    receiver: &mut DynSubscriber<'static, policy::CommsMessage>,
    expected_device: policy::DeviceId,
) {
    let WaitResult::Message(policy::CommsMessage {
        data: policy::CommsData::ConsumerDisconnected(device),
    }) = receiver.next_message().await
    else {
        panic!("Expected ConsumerDisconnected event");
    };
    assert_eq!(device, expected_device);
}

pub async fn assert_consumer_connected(
    receiver: &mut DynSubscriber<'static, policy::CommsMessage>,
    expected_device: policy::DeviceId,
    expected_capability: ConsumerPowerCapability,
) {
    let WaitResult::Message(policy::CommsMessage {
        data: policy::CommsData::ConsumerConnected(device, capability),
    }) = receiver.next_message().await
    else {
        panic!("Expected ConsumerConnected event");
    };
    assert_eq!(device, expected_device);
    assert_eq!(capability, expected_capability);
}

pub async fn assert_provider_disconnected(
    receiver: &mut DynSubscriber<'static, policy::CommsMessage>,
    expected_device: policy::DeviceId,
) {
    let WaitResult::Message(policy::CommsMessage {
        data: policy::CommsData::ProviderDisconnected(device),
    }) = receiver.next_message().await
    else {
        panic!("Expected ProviderDisconnected event");
    };
    assert_eq!(device, expected_device);
}

pub async fn assert_provider_connected(
    receiver: &mut DynSubscriber<'static, policy::CommsMessage>,
    expected_device: policy::DeviceId,
    expected_capability: ProviderPowerCapability,
) {
    let WaitResult::Message(policy::CommsMessage {
        data: policy::CommsData::ProviderConnected(device, capability),
    }) = receiver.next_message().await
    else {
        panic!("Expected ProviderConnected event");
    };
    assert_eq!(device, expected_device);
    assert_eq!(capability, expected_capability);
}

pub async fn assert_unconstrained(
    receiver: &mut DynSubscriber<'static, policy::CommsMessage>,
    expected_state: UnconstrainedState,
) {
    let WaitResult::Message(policy::CommsMessage {
        data: policy::CommsData::Unconstrained(state),
    }) = receiver.next_message().await
    else {
        panic!("Expected Unconstrained event");
    };
    assert_eq!(state, expected_state);
}

pub fn assert_no_event(receiver: &mut DynSubscriber<'static, policy::CommsMessage>) {
    assert!(receiver.try_next_message().is_none());
}
