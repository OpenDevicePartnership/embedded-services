use embassy_executor::{Executor, Spawner};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{self, Channel},
    mutex::Mutex,
};
use embassy_time::{self as _, Timer};
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::{GlobalRawMutex, event::NoopSender, named::Named};
use log::*;
use power_policy_interface::{
    capability::{ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    charger, psu,
};
use power_policy_interface::{
    charger::Charger,
    psu::{Error, Psu},
};
use power_policy_service::{
    charger::ChargerEventReceivers, psu::PsuEventReceivers, service::registration::ArrayRegistration,
};
use static_cell::StaticCell;

type ServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, DeviceType, 2, NoopSender, 1, ChargerType, 1>,
    >,
>;

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
    sender: channel::DynamicSender<'a, power_policy_interface::psu::event::EventData>,
    state: psu::State,
    name: &'static str,
}

impl<'a> ExampleDevice<'a> {
    fn new(
        name: &'static str,
        sender: channel::DynamicSender<'a, power_policy_interface::psu::event::EventData>,
    ) -> Self {
        Self {
            name,
            sender,
            state: Default::default(),
        }
    }

    pub async fn simulate_attach(&mut self) {
        self.sender
            .send(power_policy_interface::psu::event::EventData::Attached)
            .await;
    }

    pub async fn simulate_update_consumer_power_capability(&mut self, capability: Option<ConsumerPowerCapability>) {
        self.sender
            .send(power_policy_interface::psu::event::EventData::UpdatedConsumerCapability(capability))
            .await;
    }

    pub async fn simulate_detach(&mut self) {
        self.sender
            .send(power_policy_interface::psu::event::EventData::Detached)
            .await;
    }

    pub async fn simulate_update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) {
        self.sender
            .send(power_policy_interface::psu::event::EventData::RequestedProviderCapability(capability))
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

    fn state(&self) -> &psu::State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut psu::State {
        &mut self.state
    }
}

impl Named for ExampleDevice<'_> {
    fn name(&self) -> &'static str {
        self.name
    }
}

type DeviceType = Mutex<GlobalRawMutex, ExampleDevice<'static>>;

struct ExampleCharger<'a> {
    sender: channel::DynamicSender<'a, power_policy_interface::charger::event::EventData>,
    state: charger::State,
}

impl<'a> ExampleCharger<'a> {
    fn new(sender: channel::DynamicSender<'a, power_policy_interface::charger::event::EventData>) -> Self {
        Self {
            sender,
            state: charger::State::default(),
        }
    }

    fn assert_state(&self, internal_state: charger::InternalState, capability: Option<ConsumerPowerCapability>) {
        assert_eq!(*self.state.internal_state(), internal_state);
        assert_eq!(*self.state.capability(), capability);
    }

    pub async fn simulate_psu_state_change(&mut self, psu_state: charger::PsuState) {
        self.sender.send(charger::EventData::PsuStateChange(psu_state)).await;
        self.state_mut().on_psu_state_change(psu_state).unwrap();
    }

    pub fn simulate_timeout(&mut self) {
        self.state_mut().on_timeout();
    }

    pub async fn simulate_check_ready(&mut self) {
        self.is_ready().await.unwrap();
    }

    pub async fn simulate_init_request(&mut self) {
        self.init_charger().await.unwrap();
    }
}

impl<'a> embedded_batteries_async::charger::ErrorType for ExampleCharger<'a> {
    type Error = core::convert::Infallible;
}

impl<'a> embedded_batteries_async::charger::Charger for ExampleCharger<'a> {
    async fn charging_current(&mut self, current: MilliAmps) -> Result<MilliAmps, Self::Error> {
        Ok(current)
    }

    async fn charging_voltage(&mut self, voltage: MilliVolts) -> Result<MilliVolts, Self::Error> {
        Ok(voltage)
    }
}

impl<'a> charger::Charger for ExampleCharger<'a> {
    type ChargerError = core::convert::Infallible;

    async fn init_charger(&mut self) -> Result<charger::PsuState, Self::ChargerError> {
        info!("Charger init");
        self.state_mut().on_initialized(charger::PsuState::Detached).unwrap();
        Ok(charger::PsuState::Detached)
    }

    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargerError>> {
        info!("Charger attach: {:?}", capability);
        self.state_mut().on_policy_attach(capability);
        async { Ok(()) }
    }

    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargerError>> {
        info!("Charger detach");
        self.state_mut().on_policy_detach();
        async { Ok(()) }
    }

    async fn is_ready(&mut self) -> Result<(), Self::ChargerError> {
        info!("Charger check ready");
        self.state_mut().on_ready_success();
        Ok(())
    }

    fn state(&self) -> &charger::State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut charger::State {
        &mut self.state
    }
}

type ChargerType = Mutex<GlobalRawMutex, ExampleCharger<'static>>;

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_interface::psu::event::EventData, 4>> =
        StaticCell::new();
    let device0_event_channel = DEVICE0_EVENT_CHANNEL.init(Channel::new());
    static DEVICE0: StaticCell<DeviceType> = StaticCell::new();
    let device0 = DEVICE0.init(Mutex::new(ExampleDevice::new(
        "Device 0",
        device0_event_channel.dyn_sender(),
    )));

    info!("Creating device 1");
    static DEVICE1_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, power_policy_interface::psu::event::EventData, 4>> =
        StaticCell::new();
    let device1_event_channel = DEVICE1_EVENT_CHANNEL.init(Channel::new());
    static DEVICE1: StaticCell<DeviceType> = StaticCell::new();
    let device1 = DEVICE1.init(Mutex::new(ExampleDevice::new(
        "Device 1",
        device1_event_channel.dyn_sender(),
    )));

    info!("Creating charger 0");
    static CHARGER0_EVENT_CHANNEL: StaticCell<
        Channel<NoopRawMutex, power_policy_interface::charger::event::EventData, 4>,
    > = StaticCell::new();
    let charger0_event_channel = CHARGER0_EVENT_CHANNEL.init(Channel::new());
    static CHARGER0: StaticCell<ChargerType> = StaticCell::new();
    let charger0 = CHARGER0.init(Mutex::new(ExampleCharger::new(charger0_event_channel.dyn_sender())));

    let registration = ArrayRegistration {
        psus: [device0, device1],
        service_senders: [NoopSender],
        chargers: [charger0],
    };

    static SERVICE: StaticCell<ServiceType> = StaticCell::new();
    let service = SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        registration,
        power_policy_service::service::config::Config::default(),
    )));

    spawner.spawn(
        power_policy_task(
            PsuEventReceivers::new(
                [device0, device1],
                [
                    device0_event_channel.dyn_receiver(),
                    device1_event_channel.dyn_receiver(),
                ],
            ),
            ChargerEventReceivers::new([charger0], [charger0_event_channel.dyn_receiver()]),
            service,
        )
        .expect("Failed to create power policy task"),
    );

    // Check ready charger 0, should transition to Powered(Init)
    info!("Charger 0 check ready");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_check_ready().await;
        chrg0.assert_state(charger::InternalState::Powered(charger::PoweredSubstate::Init), None);
    }

    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Check ready charger 0, should transition to Powered(PsuDetached)
    info!("Charger 0 init");
    {
        let mut chrg0 = charger0.lock().await;
        // For production code, use more robust error handling (eg. retries) instead of blowing up.
        chrg0.simulate_init_request().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuDetached),
            None,
        );
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

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
    {
        // Simulate PSU attach
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_psu_state_change(charger::PsuState::Attached).await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            Some(ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: ConsumerFlags::none().with_unconstrained_power(),
            }),
        );
    }
    // Plug in device 1, should become current consumer
    // Charger should detach from device 0 and attach to device 1 with higher power
    info!("Connecting device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_attach().await;
        dev1.simulate_update_consumer_power_capability(Some(HIGH_POWER.into()))
            .await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    {
        let chrg0 = charger0.lock().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            Some(HIGH_POWER.into()),
        );
    }

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
    // Charger should detach from device 1 and attach to device 0 with lower power
    info!("Unplugging device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    {
        let chrg0 = charger0.lock().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            Some(LOW_POWER.into()),
        );
    }

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

    // Charger should still have device 1 capability
    {
        let chrg0 = charger0.lock().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            Some(HIGH_POWER.into()),
        );
    }

    // Detach device 1, no consumers available
    // Charger should detach and clear capability
    info!("Detaching device 1");
    {
        let mut dev1 = device1.lock().await;
        dev1.simulate_detach().await;
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    {
        let chrg0 = charger0.lock().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            None,
        );
    }

    // Simulate charger PSU detach, charger should transition to PsuDetached
    info!("Simulating charger PSU detach");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_psu_state_change(charger::PsuState::Detached).await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuDetached),
            None,
        );
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Simulate charger PSU reattach
    info!("Simulating charger PSU reattach");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_psu_state_change(charger::PsuState::Attached).await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuAttached),
            None,
        );
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Simulate charger timeout, should transition to Unpowered
    info!("Simulating charger timeout");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_timeout();
        chrg0.assert_state(charger::InternalState::Unpowered, None);
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    // Recover charger: CheckReady -> Init -> PsuDetached
    info!("Recovering charger: CheckReady");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_check_ready().await;
        chrg0.assert_state(charger::InternalState::Powered(charger::PoweredSubstate::Init), None);
    }
    Timer::after_millis(PER_CALL_DELAY_MS).await;

    info!("Recovering charger: InitRequest");
    {
        let mut chrg0 = charger0.lock().await;
        chrg0.simulate_init_request().await;
        chrg0.assert_state(
            charger::InternalState::Powered(charger::PoweredSubstate::PsuDetached),
            None,
        );
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
    psu_events: PsuEventReceivers<
        'static,
        2,
        DeviceType,
        channel::DynamicReceiver<'static, power_policy_interface::psu::event::EventData>,
    >,
    charger_events: ChargerEventReceivers<
        'static,
        1,
        ChargerType,
        channel::DynamicReceiver<'static, power_policy_interface::charger::event::EventData>,
    >,
    power_policy: &'static ServiceType,
) {
    power_policy_service::service::task::task(psu_events, charger_events, power_policy).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(run(spawner).expect("Failed to create run task"));
    });
}
