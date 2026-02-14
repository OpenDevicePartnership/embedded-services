#![allow(clippy::unwrap_used)]
use embassy_sync::{channel::DynamicSender, signal::Signal};
use embedded_services::{GlobalRawMutex, info};
use power_policy_service::{
    capability::{ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
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

pub struct Mock<'a> {
    sender: DynamicSender<'a, EventData>,
    fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    // Internal state
    pub state: State,
    name: &'static str,
}

impl<'a> Mock<'a> {
    pub fn new(
        name: &'static str,
        sender: DynamicSender<'a, EventData>,
        fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    ) -> Self {
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

    pub async fn simulate_consumer_connection(&mut self, capability: PowerCapability) {
        self.sender.send(EventData::Attached).await;

        let capability = Some(ConsumerPowerCapability {
            capability,
            flags: ConsumerFlags::none(),
        });
        self.sender.send(EventData::UpdatedConsumerCapability(capability)).await;
    }

    pub async fn simulate_detach(&mut self) {
        self.sender.send(EventData::Detached).await;
    }
}

impl Psu for Mock<'_> {
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

    fn state(&mut self) -> &mut State {
        &mut self.state
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
