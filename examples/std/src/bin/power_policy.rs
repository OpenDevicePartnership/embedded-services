use embassy_executor::{Executor, Spawner};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{self, Channel},
    mutex::Mutex,
    pubsub::PubSubChannel,
};
use embassy_time::{self as _, Timer};
use embedded_services::{GlobalRawMutex, broadcaster::immediate as broadcaster};
use log::*;
use power_policy_service::capability::{
    ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability,
};
use power_policy_service::psu::{Error, Psu};
use static_cell::StaticCell;

const LOW_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

const HIGH_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

const DEVICE0_ID: power_policy_service::psu::DeviceId = power_policy_service::psu::DeviceId(0);
const DEVICE1_ID: power_policy_service::psu::DeviceId = power_policy_service::psu::DeviceId(1);

const PER_CALL_DELAY_MS: u64 = 1000;

struct ExampleDevice<'a> {
    sender: channel::DynamicSender<'a, power_policy_service::psu::event::RequestData>,
}

impl<'a> ExampleDevice<'a> {
    fn new(sender: channel::DynamicSender<'a, power_policy_service::psu::event::RequestData>) -> Self {
        Self { sender }
    }

    pub async fn simulate_attach(&mut self) {
        self.sender
            .send(power_policy_service::psu::event::RequestData::Attached)
            .await;
    }

    pub async fn simulate_update_consumer_power_capability(&mut self, capability: Option<ConsumerPowerCapability>) {
        self.sender
            .send(power_policy_service::psu::event::RequestData::UpdatedConsumerCapability(capability))
            .await;
    }

    pub async fn simulate_detach(&mut self) {
        self.sender
            .send(power_policy_service::psu::event::RequestData::Detached)
            .await;
    }

    pub async fn simulate_update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) {
        self.sender
            .send(power_policy_service::psu::event::RequestData::RequestedProviderCapability(capability))
            .await
    }
}

impl Psu for ExampleDevice<'_> {
    async fn disconnect(&mut self) -> Result<(), Error> {
        debug!("ExampleDevice disconnect");
        Ok(())
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        debug!("ExampleDevice connect_provider with {capability:?}");
        Ok(())
    }

    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        debug!("ExampleDevice connect_consumer with {capability:?}");
        Ok(())
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_service::psu::event::RequestData, 4>> =
        StaticCell::new();
    let device0_event_channel = DEVICE0_EVENT_CHANNEL.init(Channel::new());
    static DEVICE0: StaticCell<Mutex<GlobalRawMutex, ExampleDevice>> = StaticCell::new();
    let device0 = DEVICE0.init(Mutex::new(ExampleDevice::new(device0_event_channel.dyn_sender())));
    static DEVICE0_REGISTRATION: StaticCell<
        power_policy_service::psu::RegistrationEntry<
            'static,
            Mutex<GlobalRawMutex, ExampleDevice>,
            channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
        >,
    > = StaticCell::new();
    let device0_registration = DEVICE0_REGISTRATION.init(power_policy_service::psu::RegistrationEntry::new(
        DEVICE0_ID,
        device0,
        device0_event_channel.dyn_receiver(),
    ));

    info!("Creating device 1");
    static DEVICE1_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_service::psu::event::RequestData, 4>> =
        StaticCell::new();
    let device1_event_channel = DEVICE1_EVENT_CHANNEL.init(Channel::new());
    static DEVICE1: StaticCell<Mutex<GlobalRawMutex, ExampleDevice>> = StaticCell::new();
    let device1 = DEVICE1.init(Mutex::new(ExampleDevice::new(device1_event_channel.dyn_sender())));
    static DEVICE1_REGISTRATION: StaticCell<
        power_policy_service::psu::RegistrationEntry<
            'static,
            Mutex<GlobalRawMutex, ExampleDevice>,
            channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
        >,
    > = StaticCell::new();
    let device1_registration = DEVICE1_REGISTRATION.init(power_policy_service::psu::RegistrationEntry::new(
        DEVICE1_ID,
        device1,
        device1_event_channel.dyn_receiver(),
    ));

    static SERVICE_CONTEXT: StaticCell<
        power_policy_service::service::context::Context<
            Mutex<GlobalRawMutex, ExampleDevice<'static>>,
            channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
        >,
    > = StaticCell::new();
    let service_context = SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

    service_context.register_psu(device0_registration).unwrap();
    service_context.register_psu(device1_registration).unwrap();

    static SERVICE: StaticCell<
        power_policy_service::service::Service<
            Mutex<GlobalRawMutex, ExampleDevice<'static>>,
            channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
        >,
    > = StaticCell::new();
    let service = SERVICE.init(power_policy_service::service::Service::new(
        service_context,
        power_policy_service::service::config::Config::default(),
    ));

    spawner.must_spawn(power_policy_task(service));
    spawner.must_spawn(receiver_task(service));

    // Plug in device 0, should become current consumer
    info!("Connecting device 0");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_attach().await;
        dev0.simulate_update_consumer_power_capability(Some(ConsumerPowerCapability {
            capability: LOW_POWER,
            flags: ConsumerFlags::none().with_unconstrained_power(),
        }))
        .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Plug in device 1, should become current consumer
    info!("Connecting device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_attach().await;
        dev1.simulate_update_consumer_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Unplug device 0, device 1 should remain current consumer
    info!("Unplugging device 0");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Plug in device 0, device 1 should remain current consumer
    info!("Connecting device 0");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_attach().await;
        dev0.simulate_update_consumer_power_capability(Some(LOW_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Unplug device 1, device 0 should become current consumer
    info!("Unplugging device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Replug device 1, device 1 becomes current consumer
    info!("Connecting device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_attach().await;
        dev1.simulate_update_consumer_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Detach consumer device 0, device 1 should remain current consumer
    // Device 0 should not be able to consume after device 1 is unplugged
    info!("Detach device 0");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_update_consumer_power_capability(None).await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Switch device 0 to provider
    info!("Device 0 switch to provider");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_update_requested_provider_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Attach device 1 and request provider
    info!("Device 1 attach and requesting provider");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_attach().await;
        dev1.simulate_update_requested_provider_power_capability(Some(LOW_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Provider upgrade should fail because device 0 is already connected at high power
    info!("Device 1 attempting provider upgrade");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_update_requested_provider_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Disconnect device 0
    info!("Device 0 disconnecting");
    {
        let mut dev0 = device0.lock().await;
        dev0.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Provider upgrade should succeed now
    info!("Device 1 attempting provider upgrade");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_update_requested_provider_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;
}

#[embassy_executor::task]
async fn receiver_task(
    service: &'static power_policy_service::service::Service<
        'static,
        Mutex<GlobalRawMutex, ExampleDevice<'static>>,
        channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
    >,
) {
    static CHANNEL: StaticCell<
        PubSubChannel<NoopRawMutex, power_policy_service::service::event::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();
    let channel = CHANNEL.init(PubSubChannel::new());

    let publisher = channel.dyn_immediate_publisher();
    let mut subscriber = channel.dyn_subscriber().unwrap();

    static RECEIVER: StaticCell<broadcaster::Receiver<'static, power_policy_service::service::event::CommsMessage>> =
        StaticCell::new();
    let receiver = RECEIVER.init(broadcaster::Receiver::new(publisher));

    service.context.register_message_receiver(receiver).unwrap();

    loop {
        match subscriber.next_message().await {
            embassy_sync::pubsub::WaitResult::Message(msg) => {
                info!("Received message: {msg:?}");
            }
            embassy_sync::pubsub::WaitResult::Lagged(count) => {
                warn!("Lagged messages: {count}");
            }
        }
    }
}

#[embassy_executor::task]
async fn power_policy_task(
    power_policy: &'static power_policy_service::service::Service<
        'static,
        Mutex<GlobalRawMutex, ExampleDevice<'static>>,
        channel::DynamicReceiver<'static, power_policy_service::psu::event::RequestData>,
    >,
) {
    power_policy_service::service::task::task(power_policy).await.unwrap();
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}
