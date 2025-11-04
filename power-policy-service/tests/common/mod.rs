use embassy_futures::{
    join::join,
    select::{Either, select},
};
use embassy_sync::{
    channel::{Channel, DynamicReceiver, DynamicSender},
    mutex::Mutex,
    signal::Signal,
};
use embassy_time::{Duration, with_timeout};
use embedded_services::{
    GlobalRawMutex,
    power::policy::{self, DeviceId, PowerCapability, device, policy::RequestData},
};
use power_policy_service::PowerPolicy;

pub mod mock;

use mock::Mock;
use static_cell::StaticCell;

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

async fn power_policy_task(
    completion_signal: &'static Signal<GlobalRawMutex, ()>,
    power_policy: &'static PowerPolicy<
        Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
        DynamicReceiver<'static, RequestData>,
    >,
) {
    loop {
        match select(power_policy.process(), completion_signal.wait()).await {
            Either::First(result) => result.unwrap(),
            Either::Second(_) => {
                break;
            }
        }
    }
}

pub async fn run_test<F: Future<Output = ()>>(
    timeout: Duration,
    test: impl FnOnce(
        &'static Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
        &'static Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
    ) -> F,
) {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();
    embedded_services::init().await;

    static DEVICE0_EVENT_CHANNEL: StaticCell<Channel<GlobalRawMutex, RequestData, EVENT_CHANNEL_SIZE>> =
        StaticCell::new();
    let device0_event_channel = DEVICE0_EVENT_CHANNEL.init(Channel::new());
    let device0_sender = device0_event_channel.dyn_sender();
    let device0_receiver = device0_event_channel.dyn_receiver();

    static DEVICE0: StaticCell<Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>> = StaticCell::new();
    let device0 = DEVICE0.init(Mutex::new(Mock::new(device0_sender)));

    static DEVICE0_REGISTRATION: StaticCell<
        device::Device<
            'static,
            Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
            DynamicReceiver<'static, RequestData>,
        >,
    > = StaticCell::new();
    let device0_registration = DEVICE0_REGISTRATION.init(device::Device::new(DeviceId(0), device0, device0_receiver));

    policy::register_device(device0_registration).await.unwrap();

    static DEVICE1_EVENT_CHANNEL: StaticCell<Channel<GlobalRawMutex, RequestData, EVENT_CHANNEL_SIZE>> =
        StaticCell::new();
    let device1_event_channel = DEVICE1_EVENT_CHANNEL.init(Channel::new());
    let device1_sender = device1_event_channel.dyn_sender();
    let device1_receiver = device1_event_channel.dyn_receiver();

    static DEVICE1: StaticCell<Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>> = StaticCell::new();
    let device1 = DEVICE1.init(Mutex::new(Mock::new(device1_sender)));

    static DEVICE1_REGISTRATION: StaticCell<
        device::Device<
            'static,
            Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
            DynamicReceiver<'static, RequestData>,
        >,
    > = StaticCell::new();
    let device1_registration = DEVICE1_REGISTRATION.init(device::Device::new(DeviceId(1), device1, device1_receiver));

    policy::register_device(device1_registration).await.unwrap();

    static POWER_POLICY: StaticCell<
        PowerPolicy<
            Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
            DynamicReceiver<'static, RequestData>,
        >,
    > = StaticCell::new();
    let power_policy = POWER_POLICY.init(power_policy_service::PowerPolicy::create(Default::default()).unwrap());

    static COMPLETION_SIGNAL: StaticCell<Signal<GlobalRawMutex, ()>> = StaticCell::new();
    let completion_signal = COMPLETION_SIGNAL.init(Signal::new());

    with_timeout(
        timeout,
        join(power_policy_task(completion_signal, power_policy), async {
            test(device0, device1).await;
            completion_signal.signal(());
        }),
    )
    .await
    .unwrap();
}
