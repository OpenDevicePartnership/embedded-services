use embassy_executor::{Executor, Spawner};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{self, Channel},
    mutex::Mutex,
};
use embassy_time::{self as _, Timer};
use embedded_services::GlobalRawMutex;
use log::*;
use power_policy_service::psu::{Error, Psu};
use power_policy_service::{
    capability::{ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    psu,
};
use static_cell::StaticCell;

const LOW_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

const HIGH_POWER: PowerCapability = PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

const PER_CALL_DELAY_MS: u64 = 1000;

struct ExampleDevice<'a> {
    sender: channel::DynamicSender<'a, power_policy_service::psu::event::EventData>,
    state: psu::State,
    name: &'static str,
}

impl<'a> ExampleDevice<'a> {
    fn new(
        name: &'static str,
        sender: channel::DynamicSender<'a, power_policy_service::psu::event::EventData>,
    ) -> Self {
        Self {
            name,
            sender,
            state: Default::default(),
        }
    }

    pub async fn simulate_attach(&mut self) {
        self.sender
            .send(power_policy_service::psu::event::EventData::Attached)
            .await;
    }

    pub async fn simulate_update_consumer_power_capability(&mut self, capability: Option<ConsumerPowerCapability>) {
        self.sender
            .send(power_policy_service::psu::event::EventData::UpdatedConsumerCapability(
                capability,
            ))
            .await;
    }

    pub async fn simulate_detach(&mut self) {
        self.sender
            .send(power_policy_service::psu::event::EventData::Detached)
            .await;
    }

    pub async fn simulate_update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) {
        self.sender
            .send(power_policy_service::psu::event::EventData::RequestedProviderCapability(capability))
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

    fn state(&mut self) -> &mut psu::State {
        &mut self.state
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

type DeviceType = Mutex<GlobalRawMutex, ExampleDevice<'static>>;

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_service::psu::event::EventData, 4>> =
        StaticCell::new();
    let device0_event_channel = DEVICE0_EVENT_CHANNEL.init(Channel::new());
    static DEVICE0: StaticCell<DeviceType> = StaticCell::new();
    let device0 = DEVICE0.init(Mutex::new(ExampleDevice::new(
        "Device 0",
        device0_event_channel.dyn_sender(),
    )));

    info!("Creating device 1");
    static DEVICE1_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_service::psu::event::EventData, 4>> =
        StaticCell::new();
    let device1_event_channel = DEVICE1_EVENT_CHANNEL.init(Channel::new());
    static DEVICE1: StaticCell<DeviceType> = StaticCell::new();
    let device1 = DEVICE1.init(Mutex::new(ExampleDevice::new(
        "Device 1",
        device1_event_channel.dyn_sender(),
    )));

    static SERVICE_CONTEXT: StaticCell<power_policy_service::service::context::Context> = StaticCell::new();
    let service_context = SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

    static POWER_POLICY_PSU_REGISTRATION: StaticCell<[&DeviceType; 2]> = StaticCell::new();
    let psu_registration = POWER_POLICY_PSU_REGISTRATION.init([device0, device1]);

    static SERVICE: StaticCell<Mutex<GlobalRawMutex, power_policy_service::service::Service<'static, DeviceType>>> =
        StaticCell::new();
    let service = SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        psu_registration.as_slice(),
        service_context,
        power_policy_service::service::config::Config::default(),
    )));

    spawner.must_spawn(power_policy_task(
        psu::event::EventReceivers::new(
            [device0, device1],
            [
                device0_event_channel.dyn_receiver(),
                device1_event_channel.dyn_receiver(),
            ],
        ),
        service,
    ));

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
async fn power_policy_task(
    psu_events: crate::psu::event::EventReceivers<'static, 2, DeviceType>,
    power_policy: &'static Mutex<GlobalRawMutex, power_policy_service::service::Service<'static, DeviceType>>,
) {
    power_policy_service::service::task::task(psu_events, power_policy).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}
