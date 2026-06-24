//! Charger mock implementation for testing

use std::collections::VecDeque;

use embassy_sync::mutex::Mutex;
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::{GlobalRawMutex, event::NonBlockingSender};
use power_policy_interface::{capability::ConsumerPowerCapability, charger};

/// Contains a charger function call and its arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    InitCharger,
    AttachHandler(ConsumerPowerCapability),
    DetachHandler,
    IsReady,
    ChargingCurrent(MilliAmps),
    ChargingVoltage(MilliVolts),
}

/// Mock charger for use in tests
pub struct Mock<S: NonBlockingSender<charger::event::EventData>> {
    sender: S,
    state: charger::State,
    /// Recorded function calls
    pub fn_calls: VecDeque<FnCall>,
    /// Next results to return for [`charger::Charger::init_charger`]
    pub next_result_init_charger: VecDeque<Result<charger::PsuState, core::convert::Infallible>>,
    /// Next results to return for [`charger::Charger::attach_handler`]
    pub next_result_attach_handler: VecDeque<Result<(), core::convert::Infallible>>,
    /// Next results to return for [`charger::Charger::detach_handler`]
    pub next_result_detach_handler: VecDeque<Result<(), core::convert::Infallible>>,
    /// Next results to return for [`charger::Charger::is_ready`]
    pub next_result_is_ready: VecDeque<Result<(), core::convert::Infallible>>,
    /// Next results to return for [`embedded_batteries_async::charger::Charger::charging_current`]
    pub next_result_charging_current: VecDeque<Result<MilliAmps, core::convert::Infallible>>,
    /// Next results to return for [`embedded_batteries_async::charger::Charger::charging_voltage`]
    pub next_result_charging_voltage: VecDeque<Result<MilliVolts, core::convert::Infallible>>,
}

impl<S: NonBlockingSender<charger::event::EventData>> Mock<S> {
    pub fn new(sender: S) -> Self {
        Self {
            sender,
            state: charger::State::default(),
            fn_calls: VecDeque::new(),
            next_result_init_charger: VecDeque::new(),
            next_result_attach_handler: VecDeque::new(),
            next_result_detach_handler: VecDeque::new(),
            next_result_is_ready: VecDeque::new(),
            next_result_charging_current: VecDeque::new(),
            next_result_charging_voltage: VecDeque::new(),
        }
    }

    pub fn assert_state(&self, internal_state: charger::InternalState, capability: Option<ConsumerPowerCapability>) {
        assert_eq!(*self.state.internal_state(), internal_state);
        assert_eq!(*self.state.capability(), capability);
    }

    pub async fn simulate_psu_state_change(&mut self, psu_state: charger::PsuState) {
        self.sender
            .try_send(charger::EventData::PsuStateChange(psu_state))
            .unwrap();
    }
}

impl<S: NonBlockingSender<charger::event::EventData>> embedded_batteries_async::charger::ErrorType for Mock<S> {
    type Error = core::convert::Infallible;
}

impl<S: NonBlockingSender<charger::event::EventData>> embedded_batteries_async::charger::Charger for Mock<S> {
    async fn charging_current(&mut self, current: MilliAmps) -> Result<MilliAmps, Self::Error> {
        self.fn_calls.push_back(FnCall::ChargingCurrent(current));
        self.next_result_charging_current
            .pop_front()
            .expect("next_result_charging_current not set")
    }

    async fn charging_voltage(&mut self, voltage: MilliVolts) -> Result<MilliVolts, Self::Error> {
        self.fn_calls.push_back(FnCall::ChargingVoltage(voltage));
        self.next_result_charging_voltage
            .pop_front()
            .expect("next_result_charging_voltage not set")
    }
}

impl<S: NonBlockingSender<charger::event::EventData>> charger::Charger for Mock<S> {
    type ChargerError = core::convert::Infallible;

    async fn init_charger(&mut self) -> Result<charger::PsuState, Self::ChargerError> {
        self.fn_calls.push_back(FnCall::InitCharger);
        self.next_result_init_charger
            .pop_front()
            .expect("next_result_init_charger not set")
    }

    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargerError>> {
        self.fn_calls.push_back(FnCall::AttachHandler(capability));
        let result = self
            .next_result_attach_handler
            .pop_front()
            .expect("next_result_attach_handler not set");
        async move { result }
    }

    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargerError>> {
        self.fn_calls.push_back(FnCall::DetachHandler);
        let result = self
            .next_result_detach_handler
            .pop_front()
            .expect("next_result_detach_handler not set");
        async move { result }
    }

    async fn is_ready(&mut self) -> Result<(), Self::ChargerError> {
        self.fn_calls.push_back(FnCall::IsReady);
        self.next_result_is_ready
            .pop_front()
            .expect("next_result_is_ready not set")
    }

    fn state(&self) -> &charger::State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut charger::State {
        &mut self.state
    }
}

pub type ChargerType<S> = Mutex<GlobalRawMutex, Mock<S>>;
