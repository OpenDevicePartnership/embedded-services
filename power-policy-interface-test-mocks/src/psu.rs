//! PSU mock implementation for testing

use std::collections::VecDeque;

use embedded_services::{event::NonBlockingSender, named::Named};
use power_policy_interface::{
    capability::{
        ConsumerDisconnect, ConsumerPowerCapability, PowerCapability, ProviderFlags, ProviderPowerCapability,
    },
    psu::{Error, Psu, State, event::EventData},
};

/// Contains a PSU function call and its arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
}

/// Mock PSU for use in tests
pub struct Mock<S: NonBlockingSender<EventData>> {
    sender: S,
    name: &'static str,
    pub state: State,
    /// Recorded function calls
    pub fn_calls: VecDeque<FnCall>,
    /// Next results to return for [`Psu::connect_consumer`]
    pub next_result_connect_consumer: VecDeque<Result<(), Error>>,
    /// Next results to return for [`Psu::connect_provider`]
    pub next_result_connect_provider: VecDeque<Result<(), Error>>,
    /// Next results to return for [`Psu::disconnect`]
    pub next_result_disconnect: VecDeque<Result<(), Error>>,
}

impl<S: NonBlockingSender<EventData>> Mock<S> {
    pub fn new(name: &'static str, sender: S) -> Self {
        Self {
            name,
            sender,
            state: Default::default(),
            fn_calls: VecDeque::new(),
            next_result_connect_consumer: VecDeque::new(),
            next_result_connect_provider: VecDeque::new(),
            next_result_disconnect: VecDeque::new(),
        }
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

impl<S: NonBlockingSender<EventData>> Psu for Mock<S> {
    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), Error> {
        self.fn_calls.push_back(FnCall::ConnectConsumer(capability));
        let result = self
            .next_result_connect_consumer
            .pop_front()
            .expect("next_result_connect_consumer not set");
        if result.is_ok() {
            self.state.connect_consumer(capability).unwrap();
        }
        result
    }

    async fn connect_provider(&mut self, capability: ProviderPowerCapability) -> Result<(), Error> {
        self.fn_calls.push_back(FnCall::ConnectProvider(capability));
        let result = self
            .next_result_connect_provider
            .pop_front()
            .expect("next_result_connect_provider not set");
        if result.is_ok() {
            self.state.connect_provider(capability).unwrap();
        }
        result
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        self.fn_calls.push_back(FnCall::Disconnect);
        let result = self
            .next_result_disconnect
            .pop_front()
            .expect("next_result_disconnect not set");
        if result.is_ok() {
            self.state.disconnect(false).unwrap();
        }
        result
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<S: NonBlockingSender<EventData>> Named for Mock<S> {
    fn name(&self) -> &'static str {
        self.name
    }
}
