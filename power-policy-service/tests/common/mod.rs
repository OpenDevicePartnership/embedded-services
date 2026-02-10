#![allow(clippy::unwrap_used)]
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
use embedded_services::GlobalRawMutex;
use power_policy_service::capability::PowerCapability;
use power_policy_service::psu;
use power_policy_service::psu::DeviceId;
use power_policy_service::psu::event::RequestData;
use power_policy_service::service::Service;

pub mod mock;

use mock::Mock;
use static_cell::StaticCell;

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

async fn power_policy_task(
    completion_signal: &'static Signal<GlobalRawMutex, ()>,
    power_policy: &'static Service<
        'static,
        Mutex<GlobalRawMutex, Mock<'static, DynamicSender<'static, RequestData>>>,
        DynamicReceiver<'static, RequestData>,
    >,
) {
    while let Either::First(result) = select(power_policy.process(), completion_signal.wait()).await {
        result.unwrap();
    }
}

pub type RegistrationType = psu::RegistrationEntry<
    'static,
    Mutex<GlobalRawMutex, Mock<'static, DynamicSender<'static, RequestData>>>,
    DynamicReceiver<'static, RequestData>,
>;

pub type ServiceType = Service<
    'static,
    Mutex<GlobalRawMutex, Mock<'static, DynamicSender<'static, RequestData>>>,
    DynamicReceiver<'static, RequestData>,
>;

pub type ServiceContext = power_policy_service::service::context::Context<
    Mutex<GlobalRawMutex, Mock<'static, DynamicSender<'static, RequestData>>>,
    DynamicReceiver<'static, RequestData>,
>;

pub async fn run_test<F: Future<Output = ()>>(
    timeout: Duration,
    test: impl FnOnce(
        &'static Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
        &'static Signal<GlobalRawMutex, (usize, FnCall)>,
        &'static Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>,
        &'static Signal<GlobalRawMutex, (usize, FnCall)>,
    ) -> F,
) {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();
    embedded_services::init().await;

    static DEVICE0_EVENT_CHANNEL: StaticCell<Channel<GlobalRawMutex, RequestData, EVENT_CHANNEL_SIZE>> =
        StaticCell::new();
    let device0_event_channel = DEVICE0_EVENT_CHANNEL.init(Channel::new());
    let device0_sender = device0_event_channel.dyn_sender();
    let device0_receiver = device0_event_channel.dyn_receiver();

    static DEVICE0_SIGNAL: StaticCell<Signal<GlobalRawMutex, (usize, FnCall)>> = StaticCell::new();
    let device0_signal = DEVICE0_SIGNAL.init(Signal::new());
    static DEVICE0: StaticCell<Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>> = StaticCell::new();
    let device0 = DEVICE0.init(Mutex::new(Mock::new(device0_sender, device0_signal)));

    static DEVICE0_REGISTRATION: StaticCell<RegistrationType> = StaticCell::new();
    let device0_registration = DEVICE0_REGISTRATION.init(psu::RegistrationEntry::new(DeviceId(0), device0, device0_receiver));

    static DEVICE1_EVENT_CHANNEL: StaticCell<Channel<GlobalRawMutex, RequestData, EVENT_CHANNEL_SIZE>> =
        StaticCell::new();
    let device1_event_channel = DEVICE1_EVENT_CHANNEL.init(Channel::new());
    let device1_sender = device1_event_channel.dyn_sender();
    let device1_receiver = device1_event_channel.dyn_receiver();

    static DEVICE1_SIGNAL: StaticCell<Signal<GlobalRawMutex, (usize, FnCall)>> = StaticCell::new();
    let device1_signal = DEVICE1_SIGNAL.init(Signal::new());
    static DEVICE1: StaticCell<Mutex<GlobalRawMutex, Mock<DynamicSender<'static, RequestData>>>> = StaticCell::new();
    let device1 = DEVICE1.init(Mutex::new(Mock::new(device1_sender, device1_signal)));

    static DEVICE1_REGISTRATION: StaticCell<RegistrationType> = StaticCell::new();
    let device1_registration = DEVICE1_REGISTRATION.init(psu::RegistrationEntry::new(DeviceId(1), device1, device1_receiver));

    static SERVICE_CONTEXT: StaticCell<ServiceContext> = StaticCell::new();
    let service_context = SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

    service_context.register_psu(device0_registration).unwrap();
    service_context.register_psu(device1_registration).unwrap();

    static POWER_POLICY: StaticCell<ServiceType> = StaticCell::new();
    let power_policy = POWER_POLICY.init(power_policy_service::service::Service::new(
        service_context,
        Default::default(),
    ));

    static COMPLETION_SIGNAL: StaticCell<Signal<GlobalRawMutex, ()>> = StaticCell::new();
    let completion_signal = COMPLETION_SIGNAL.init(Signal::new());

    with_timeout(
        timeout,
        join(power_policy_task(completion_signal, power_policy), async {
            test(device0, device0_signal, device1, device1_signal).await;
            completion_signal.signal(());
        }),
    )
    .await
    .unwrap();
}
