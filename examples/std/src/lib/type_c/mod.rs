pub mod mock_controller;

pub struct DummyCharger(embedded_services::power::policy::charger::Device);
impl embedded_services::power::policy::charger::ChargerContainer for DummyCharger {
    fn get_charger(&self) -> &embedded_services::power::policy::charger::Device {
        &self.0
    }
}

pub struct DummyPowerDevice<const POLICY_CHANNEL_SIZE: usize>(
    embedded_services::power::policy::device::Device<POLICY_CHANNEL_SIZE>,
);
impl<const POLICY_CHANNEL_SIZE: usize> embedded_services::power::policy::device::DeviceContainer<POLICY_CHANNEL_SIZE>
    for DummyPowerDevice<POLICY_CHANNEL_SIZE>
{
    fn get_power_policy_device(&self) -> &embedded_services::power::policy::device::Device<POLICY_CHANNEL_SIZE> {
        &self.0
    }
}
