#![no_std]

use mimxrt600_fcb::FlexSPIFlashConfigurationBlock;
use {defmt_rtt as _, panic_probe as _};

#[unsafe(link_section = ".otfad")]
#[used]
static OTFAD: [u8; 256] = [0; 256];

#[unsafe(link_section = ".fcb")]
#[used]
static FCB: FlexSPIFlashConfigurationBlock = FlexSPIFlashConfigurationBlock::build();

#[unsafe(link_section = ".biv")]
#[used]
static BOOT_IMAGE_VERSION: u32 = 0x01000000;

#[unsafe(link_section = ".keystore")]
#[used]
static KEYSTORE: [u8; 2048] = [0; 2048];

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
