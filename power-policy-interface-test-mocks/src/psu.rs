//! PSU mock implementation for testing

use embassy_sync::signal::Signal;
use embedded_services::{GlobalRawMutex, event::NonBlockingSender, named::Named};
use log::info;
use power_policy_interface::{
    capability::{
        ConsumerDisconnect, ConsumerPowerCapability, PowerCapability, ProviderFlags, ProviderPowerCapability,
    },
    psu::{Error, Psu, State, event::EventData},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
    Reset,
}

pub struct Mock<'a, S: NonBlockingSender<EventData>> {
    sender: S,
    fn_call: &'a Signal<GlobalRawMutex, (usize, FnCall)>,
    pub state: State,
    name: &'static str,
}

impl<'a, S: NonBlockingSender<EventData>> Mock<'a, S> {
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
        self.state.attach().unwrap();
        self.sender.try_send(EventData::Attached).unwrap();
        self.state.update_consumer_power_capability(Some(capability)).unwrap();
        self.sender
            .try_send(EventData::UpdatedConsumerCapability(Some(capability)))
            .unwrap();
    }

    /// Simulate an already-attached consumer renegotiating a new power capability.
    pub async fn simulate_update_consumer_power_capability(&mut self, capability: Option<ConsumerPowerCapability>) {
        self.state.update_consumer_power_capability(capability).unwrap();
        self.sender
            .try_send(EventData::UpdatedConsumerCapability(capability))
            .unwrap();
    }

    pub async fn simulate_detach(&mut self) {
        self.state.detach();
        self.sender.try_send(EventData::Detached).unwrap();
    }

    pub async fn simulate_provider_connection(&mut self, capability: PowerCapability) {
        self.state.attach().unwrap();
        self.sender.try_send(EventData::Attached).unwrap();

        let capability = Some(ProviderPowerCapability {
            capability,
            flags: ProviderFlags::none(),
        });
        self.state
            .update_requested_provider_power_capability(capability)
            .unwrap();
        self.sender
            .try_send(EventData::RequestedProviderCapability(capability))
            .unwrap();
    }

    pub async fn simulate_disconnect(&mut self) {
        self.state.disconnect(true).unwrap();
        self.sender
            .try_send(EventData::Disconnected(ConsumerDisconnect::none()))
            .unwrap();
    }

    pub async fn simulate_update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) {
        self.state
            .update_requested_provider_power_capability(capability)
            .unwrap();
        self.sender
            .try_send(EventData::RequestedProviderCapability(capability))
            .unwrap();
    }
}

impl<'a, S: NonBlockingSender<EventData>> Psu for Mock<'a, S> {
    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        info!("Connect consumer {:#?}", capability);
        self.record_fn_call(FnCall::ConnectConsumer(capability));
        self.state.connect_consumer(capability)
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        info!("Connect provider: {:#?}", capability);
        self.record_fn_call(FnCall::ConnectProvider(capability));
        self.state.connect_provider(capability)
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        info!("Disconnect");
        self.record_fn_call(FnCall::Disconnect);
        self.state.disconnect(false)
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<'a, S: NonBlockingSender<EventData>> Named for Mock<'a, S> {
    fn name(&self) -> &'static str {
        self.name
    }
}
