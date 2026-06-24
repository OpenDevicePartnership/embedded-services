//! Charger mock implementation for testing

use embassy_sync::{channel, mutex::Mutex};
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::GlobalRawMutex;
use log::info;
use power_policy_interface::{capability::ConsumerPowerCapability, charger};

pub struct ExampleCharger<'a> {
    sender: channel::DynamicSender<'a, charger::event::EventData>,
    state: charger::State,
}

impl<'a> ExampleCharger<'a> {
    pub fn new(sender: channel::DynamicSender<'a, charger::event::EventData>) -> Self {
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
        self.sender
            .try_send(charger::EventData::PsuStateChange(psu_state))
            .unwrap();
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
