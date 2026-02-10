#![allow(clippy::unwrap_used)]
use embassy_sync::signal::Signal;
use embedded_services::{GlobalRawMutex, event, info};
use power_policy_service::{
    capability::{ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability},
    psu::{Error, InternalState, Psu, event::RequestData},
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
    Reset,
}

pub struct Mock<'a, S: event::Sender<RequestData>> {
    sender: S,
    fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    // Internal state
    pub state: InternalState,
}

impl<'a, S: event::Sender<RequestData>> Mock<'a, S> {
    pub fn new(sender: S, fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>) -> Self {
        Self {
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
        self.state.attach().unwrap();

        self.sender.send(RequestData::Attached).await;

        let capability = Some(ConsumerPowerCapability {
            capability,
            flags: ConsumerFlags::none(),
        });
        self.state.update_consumer_power_capability(capability).unwrap();
        self.sender
            .send(RequestData::UpdatedConsumerCapability(capability))
            .await;
    }

    pub async fn simulate_detach(&mut self) {
        self.state.detach();
        self.sender.send(RequestData::Detached).await;
    }
}

impl<'a, S: event::Sender<RequestData>> Psu for Mock<'a, S> {
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
}
