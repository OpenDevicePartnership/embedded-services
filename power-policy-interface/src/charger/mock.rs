use embassy_sync::mutex::Mutex;
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_services::{GlobalRawMutex, debug};

pub type ChargerType = Mutex<GlobalRawMutex, NoopCharger>;

pub struct NoopCharger(super::State);

impl NoopCharger {
    pub fn new() -> Self {
        Self(super::State::default())
    }
}
impl Default for NoopCharger {
    fn default() -> Self {
        Self::new()
    }
}

impl super::Charger for NoopCharger {
    type ChargerError = core::convert::Infallible;

    async fn init_charger(&mut self) -> Result<super::PsuState, Self::ChargerError> {
        debug!("Charger initialized");
        Ok(super::PsuState::Attached)
    }

    async fn attach_handler(
        &mut self,
        capability: crate::capability::ConsumerPowerCapability,
    ) -> Result<(), Self::ChargerError> {
        debug!("Charger recvd capability {:?}", capability);
        Ok(())
    }

    async fn detach_handler(&mut self) -> Result<(), Self::ChargerError> {
        debug!("Charger recvd detach");
        Ok(())
    }

    fn state(&self) -> &super::State {
        &self.0
    }

    fn state_mut(&mut self) -> &mut super::State {
        &mut self.0
    }
}

impl embedded_batteries_async::charger::Charger for NoopCharger {
    async fn charging_current(&mut self, current: MilliAmps) -> Result<MilliAmps, Self::Error> {
        Ok(current)
    }

    async fn charging_voltage(&mut self, voltage: MilliVolts) -> Result<MilliVolts, Self::Error> {
        Ok(voltage)
    }
}

impl embedded_batteries_async::charger::ErrorType for NoopCharger {
    type Error = core::convert::Infallible;
}
