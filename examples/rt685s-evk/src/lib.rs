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
