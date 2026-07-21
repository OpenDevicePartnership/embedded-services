//! PSU mock implementation for testing

use std::collections::VecDeque;

use embedded_services::named::Named;
use power_policy_interface::{
    capability::{
        ConsumerDisconnect, ConsumerPowerCapability, PowerCapability, ProviderFlags, ProviderPowerCapability,
    },
    psu::{self, Error, Psu, State},
};

/// Contains a PSU function call and its arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    ConnectConsumer(ConsumerPowerCapability),
    ConnectProvider(ProviderPowerCapability),
    Disconnect,
}

/// Mock PSU for use in tests
pub struct Mock<Notifier: psu::notification::Notifier> {
    notifier: Notifier,
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

impl<Notifier: psu::notification::Notifier> Mock<Notifier> {
    pub fn new(name: &'static str, notifier: Notifier) -> Self {
        Self {
            name,
            notifier,
            state: Default::default(),
            fn_calls: VecDeque::new(),
            next_result_connect_consumer: VecDeque::new(),
            next_result_connect_provider: VecDeque::new(),
            next_result_disconnect: VecDeque::new(),
        }
    }

    pub async fn simulate_consumer_connection(&mut self, capability: ConsumerPowerCapability) {
        self.state.attach().unwrap();
        self.notifier.notify_attached().await.unwrap();
        self.state.update_consumer_power_capability(Some(capability)).unwrap();
        self.notifier
            .notify_updated_consumer_capability(Some(capability))
            .await
            .unwrap();
    }

    /// Simulate an already-attached consumer renegotiating a new power capability.
    pub async fn simulate_update_consumer_power_capability(&mut self, capability: Option<ConsumerPowerCapability>) {
        self.state.update_consumer_power_capability(capability).unwrap();
        self.notifier
            .notify_updated_consumer_capability(capability)
            .await
            .unwrap();
    }

    pub async fn simulate_detach(&mut self) {
        self.state.detach();
        self.notifier.notify_detached().await.unwrap();
    }

    pub async fn simulate_provider_connection(&mut self, capability: PowerCapability) {
        self.state.attach().unwrap();
        self.notifier.notify_attached().await.unwrap();

        let capability = Some(ProviderPowerCapability {
            capability,
            flags: ProviderFlags::none(),
        });
        self.state
            .update_requested_provider_power_capability(capability)
            .unwrap();
        self.notifier
            .notify_requested_provider_capability(capability)
            .await
            .unwrap();
    }

    pub async fn simulate_disconnect(&mut self) {
        self.state.disconnect(true).unwrap();
        self.notifier
            .notify_disconnected(ConsumerDisconnect::none())
            .await
            .unwrap();
    }

    pub async fn simulate_update_requested_provider_power_capability(
        &mut self,
        capability: Option<ProviderPowerCapability>,
    ) {
        self.state
            .update_requested_provider_power_capability(capability)
            .unwrap();
        self.notifier
            .notify_requested_provider_capability(capability)
            .await
            .unwrap();
    }
}

impl<Notifier: psu::notification::Notifier> Psu for Mock<Notifier> {
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

impl<Notifier: psu::notification::Notifier> Named for Mock<Notifier> {
    fn name(&self) -> &'static str {
        self.name
    }
}
