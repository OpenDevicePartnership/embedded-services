#![no_std]

use mimxrt600_fcb::FlexSpiLutOpcode::{CMD_SDR, RADDR_SDR, READ_SDR, STOP};
use mimxrt600_fcb::FlexSpiNumPads::Single;
use mimxrt600_fcb::{
    ControllerMiscOption, FlexSPIFlashConfigurationBlock, SFlashPadType, SerialClkFreq, SerialNORType, flexspi_lut_seq,
};
use {defmt_rtt as _, panic_probe as _};

// auto-generated version information from Cargo.toml
include!(concat!(env!("OUT_DIR"), "/biv.rs"));

#[unsafe(link_section = ".otfad")]
#[used]
static OTFAD: [u8; 256] = [0; 256];

#[unsafe(link_section = ".fcb")]
#[used]
static FCB: FlexSPIFlashConfigurationBlock = FlexSPIFlashConfigurationBlock::build()
    .device_mode_cfg_enable(0)
    .wait_time_cfg_commands(0)
    .device_mode_arg([0; 4])
    .config_mode_type([0, 1, 2])
    .controller_misc_option(ControllerMiscOption(0x10))
    .sflash_pad_type(SFlashPadType::QuadPads)
    .serial_clk_freq(SerialClkFreq::SdrDdr50mhz)
    .sflash_a1_size(0x0040_0000)
    .sflash_b1_size(0)
    .lookup_table([
        // Sequence 0 - Read Data (in the default Single SPI lane mode coming out of reset)
        // 0x03 - Read Data command, 0x18 - W25Q16FW address size (24 bits)
        flexspi_lut_seq(CMD_SDR, Single, 0x03, RADDR_SDR, Single, 0x18),
        // Sequence 1 - Read 128 Data Bytes and Stop
        // 0x80 - read 128 bytes, stop
        flexspi_lut_seq(READ_SDR, Single, 0x80, STOP, Single, 0x00),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
    .serial_nor_type(SerialNORType::StandardSpi)
    .flash_state_ctx(0);

#[unsafe(link_section = ".keystore")]
#[used]
static KEYSTORE: [u8; 2048] = [0; 2048];
