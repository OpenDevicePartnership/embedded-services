#![allow(clippy::unwrap_used)]
#![allow(dead_code)]
use embassy_sync::{channel, mutex::Mutex, signal::Signal};
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::{GlobalRawMutex, event::Sender, info, named::Named};
use power_policy_interface::{
    capability::{ConsumerPowerCapability, PowerCapability, ProviderFlags, ProviderPowerCapability},
    charger,
    psu::{Error, Psu, State, event::EventData},
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
    Reset,
}

pub struct Mock<'a, S: Sender<EventData>> {
    sender: S,
    fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    // Internal state
    pub state: State,
    name: &'static str,
}

impl<'a, S: Sender<EventData>> Mock<'a, S> {
    pub fn new(name: &'static str, sender: S, fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>) -> Self {
        Self {
            name,
            sender,
            fn_call,
            state: Default::default(),
        }
    }

    fn record_fn_call(&mut self, fn_call: FnCall) {
        let num_fn_calls = self
            .fn_call
            .try_take()
            .map(|(num_fn_calls, _)| num_fn_calls)
            .unwrap_or(0);
        self.fn_call.signal((num_fn_calls + 1, fn_call));
    }

    pub async fn simulate_consumer_connection(&mut self, capability: ConsumerPowerCapability) {
        self.sender.send(EventData::Attached).await;
        self.sender
            .send(EventData::UpdatedConsumerCapability(Some(capability)))
            .await;
    }

    pub async fn simulate_detach(&mut self) {
        self.sender.send(EventData::Detached).await;
    }

    pub async fn simulate_provider_connection(&mut self, capability: PowerCapability) {
        self.sender.send(EventData::Attached).await;

        let capability = Some(ProviderPowerCapability {
            capability,
            flags: ProviderFlags::none(),
        });
        self.sender
            .send(EventData::RequestedProviderCapability(capability))
            .await;
    }

    pub async fn simulate_disconnect(&mut self) {
        self.sender.send(EventData::Disconnected).await;
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

impl<'a, S: Sender<EventData>> Psu for Mock<'a, S> {
    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        info!("Connect consumer {:#?}", capability);
        self.record_fn_call(FnCall::ConnectConsumer(capability));
        Ok(())
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        info!("Connect provider: {:#?}", capability);
        self.record_fn_call(FnCall::ConnectProvider(capability));
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        info!("Disconnect");
        self.record_fn_call(FnCall::Disconnect);
        Ok(())
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<'a, S: Sender<EventData>> Named for Mock<'a, S> {
    fn name(&self) -> &'static str {
        self.name
    }
}

pub struct ExampleCharger<'a> {
    sender: channel::DynamicSender<'a, power_policy_interface::charger::event::EventData>,
    state: charger::State,
}

impl<'a> ExampleCharger<'a> {
    pub fn new(sender: channel::DynamicSender<'a, power_policy_interface::charger::event::EventData>) -> Self {
        Self {
            sender,
            state: charger::State::default(),
        }
    }

    pub fn assert_state(&self, internal_state: charger::InternalState, capability: Option<ConsumerPowerCapability>) {
        assert_eq!(*self.state.internal_state(), internal_state);
        assert_eq!(*self.state.capability(), capability);
    }

    pub async fn simulate_psu_state_change(&self, psu_state: charger::PsuState) {
        self.sender.send(charger::EventData::PsuStateChange(psu_state)).await;
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
        Ok(charger::PsuState::Detached)
    }

    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargerError>> {
        info!("Charger attach: {:?}", capability);
        async { Ok(()) }
    }

    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargerError>> {
        info!("Charger detach");
        async { Ok(()) }
    }

    async fn is_ready(&mut self) -> Result<(), Self::ChargerError> {
        info!("Charger check ready");
        Ok(())
    }

    fn state(&self) -> &charger::State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut charger::State {
        &mut self.state
    }
}

pub type ChargerType<'a> = Mutex<GlobalRawMutex, ExampleCharger<'a>>;
