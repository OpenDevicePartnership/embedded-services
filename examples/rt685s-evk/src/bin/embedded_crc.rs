#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use crc::*;

use {defmt_rtt as _, panic_probe as _};

use defmt::{error, info};
use platform_service::embedded_crc;

const DATA: &[u8] = b"123456789";

pub const CRC_16_XMODEM_AS_U32: Algorithm<u32> = Algorithm {
    width: 16,
    poly: 0x1021,
    init: 0x0000,
    refin: false,
    refout: false,
    xorout: 0x0000,
    check: 0x31c3,
    residue: 0x0000,
};
pub const CRC_16_IBM_SDLC_AS_U32: Algorithm<u32> = Algorithm {
    width: 16,
    poly: 0x1021,
    init: 0xffff,
    refin: true,
    refout: true,
    xorout: 0xffff,
    check: 0x906e,
    residue: 0xf0b8,
};

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let p = embassy_imxrt::init(Default::default());

    // Initialize hardware CRC object
    let crc = embassy_imxrt::crc::Crc::new(p.CRC, Default::default());
    platform_service::imxrt::embedded_crc::crc_engine_init(crc);

    embedded_services::init().await;

    // u32 algorithms
    test_crc_algorithm_u32(&CRC_32_MPEG_2, "CRC_32_MPEG_2").await;
    test_crc_algorithm_u32(&CRC_32_CKSUM, "CRC_32_CKSUM").await;
    test_crc_algorithm_u32(&CRC_32_BZIP2, "CRC_32_BZIP2").await;
    test_crc_algorithm_u32(&CRC_32_ISO_HDLC, "CRC_32_ISO_HDLC").await;
    test_crc_algorithm_u32(&CRC_32_JAMCRC, "CRC_32_JAMCRC").await;

    // u16 algorithms
    test_crc_algorithm_u16(&CRC_16_ARC, "CRC_16_ARC").await;
    test_crc_algorithm_u16(&CRC_16_CMS, "CRC_16_CMS").await;
    test_crc_algorithm_u16(&CRC_16_DDS_110, "CRC_16_DDS_110").await;
    test_crc_algorithm_u16(&CRC_16_MAXIM_DOW, "CRC_16_MAXIM_DOW").await;
    test_crc_algorithm_u16(&CRC_16_MODBUS, "CRC_16_MODBUS").await;
    test_crc_algorithm_u16(&CRC_16_UMTS, "CRC_16_UMTS").await;
    test_crc_algorithm_u16(&CRC_16_USB, "CRC_16_USB").await;
    test_crc_algorithm_u16(&CRC_16_GENIBUS, "CRC_16_GENIBUS").await;
    test_crc_algorithm_u16(&CRC_16_GSM, "CRC_16_GSM").await;
    test_crc_algorithm_u16(&CRC_16_IBM_3740, "CRC_16_IBM_3740").await;
    test_crc_algorithm_u16(&CRC_16_IBM_SDLC, "CRC_16_IBM_SDLC").await;
    test_crc_algorithm_u16(&CRC_16_ISO_IEC_14443_3_A, "CRC_16_ISO_IEC_14443_3_A").await;
    test_crc_algorithm_u16(&CRC_16_KERMIT, "CRC_16_KERMIT").await;
    test_crc_algorithm_u16(&CRC_16_MCRF4XX, "CRC_16_MCRF4XX").await;
    test_crc_algorithm_u16(&CRC_16_RIELLO, "CRC_16_RIELLO").await;
    test_crc_algorithm_u16(&CRC_16_SPI_FUJITSU, "CRC_16_SPI_FUJITSU").await;
    test_crc_algorithm_u16(&CRC_16_TMS37157, "CRC_16_TMS37157").await;
    test_crc_algorithm_u16(&CRC_16_XMODEM, "CRC_16_XMODEM").await;

    // u16 algorithms with u32 type
    test_crc_algorithm_u32(&CRC_16_XMODEM_AS_U32, "CRC_16_XMODEM_AS_U32").await;
    test_crc_algorithm_u32(&CRC_16_IBM_SDLC_AS_U32, "CRC_16_IBM_SDLC_AS_U32").await;

    // The following algorithms will fail on IMXRT hardware:
    info!("\n\n--------EXPECT FAILURE FOR THE FOLLOWING ALGORITHMS ON IMXRT--------\n");
    test_crc_algorithm_u32(&CRC_31_PHILIPS, "CRC_31_PHILIPS").await;
    test_crc_algorithm_u32(&CRC_32_AUTOSAR, "CRC_32_AUTOSAR").await;
    test_crc_algorithm_u32(&CRC_32_ISCSI, "CRC_32_ISCSI").await;
    test_crc_algorithm_u32(&CRC_17_CAN_FD, "CRC_17_CAN_FD").await;
    test_crc_algorithm_u32(&CRC_21_CAN_FD, "CRC_21_CAN_FD").await;
    test_crc_algorithm_u16(&CRC_11_FLEXRAY, "CRC_11_FLEXRAY").await;
    test_crc_algorithm_u16(&CRC_12_GSM, "CRC_12_GSM").await;
}

async fn test_crc_algorithm_u32(algorithm: &'static Algorithm<u32>, name: &str) {
    // Verify that feeding the bytes separately gets the same result as the feeding the bytes all at once
    info!(
        "\n\n---------------{}---------------\ninit: 0x{:X}, refin: {}, refout: {}, xorout: 0x{:X}",
        name, algorithm.init, algorithm.refin, algorithm.refout, algorithm.xorout
    );
    info!("Expected CRC: 0x{:08X}", algorithm.check);

    // Calculate the CRC in one go
    let mut crc_handle: embedded_crc::EmbeddedCrc<u32> = embedded_crc::EmbeddedCrc::<u32>::new(algorithm);

    let oneshot_crc = match crc_handle.calculate(DATA).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC32 for {}: {}", name, e);
            return;
        }
    };

    if oneshot_crc == algorithm.check {
        info!("Algorithm check passed (CRC = 0x{:08X})", oneshot_crc);
    } else {
        error!(
            "ALGORITHM CHECK FAILED (CRC: 0x{:08X}, Expected = 0x{:08X})",
            oneshot_crc, algorithm.check
        );
    }

    // Calculate the CRC in two split function calls
    let mut crc_handle: embedded_crc::EmbeddedCrc<u32> = embedded_crc::EmbeddedCrc::<u32>::new(algorithm);

    let _first_half_crc = match crc_handle.calculate(&DATA[0..(DATA.len() / 2)]).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC32 for {}: {}", name, e);
            return;
        }
    };

    let full_crc = match crc_handle.calculate(&DATA[(DATA.len() / 2)..]).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC32 for {}: {}", name, e);
            return;
        }
    };

    if full_crc == algorithm.check {
        info!("Split calculation test passed (CRC = 0x{:08X})", full_crc);
    } else {
        error!(
            "RESUME TEST FAILED (Oneshot CRC = 0x{:08X}, Two shot CRC = 0x{:08X}, Expected CRC = 0x{:08X})",
            oneshot_crc, full_crc, algorithm.check
        );
    }
}

async fn test_crc_algorithm_u16(algorithm: &'static Algorithm<u16>, name: &str) {
    // Verify that feeding the bytes separately gets the same result as the feeding the bytes all at once
    info!(
        "\n\n---------------{}---------------\ninit: 0x{:X}, refin: {}, refout: {}, xorout: 0x{:X}",
        name, algorithm.init, algorithm.refin, algorithm.refout, algorithm.xorout
    );
    info!("Expected CRC: 0x{:04X}", algorithm.check);

    // Calculate the CRC in one go
    let mut crc_handle: embedded_crc::EmbeddedCrc<u16> = embedded_crc::EmbeddedCrc::<u16>::new(algorithm);

    let oneshot_crc = match crc_handle.calculate(DATA).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC16 for {}: {}", name, e);
            return;
        }
    };

    if oneshot_crc == algorithm.check {
        info!("Algorithm check passed (CRC = 0x{:04X})", oneshot_crc);
    } else {
        error!(
            "ALGORITHM CHECK FAILED (CRC: 0x{:04X}, Expected = 0x{:04X})",
            oneshot_crc, algorithm.check
        );
    }

    // Calculate the CRC in two split function calls
    let mut crc_handle: embedded_crc::EmbeddedCrc<u16> = embedded_crc::EmbeddedCrc::<u16>::new(algorithm);

    let _first_half_crc = match crc_handle.calculate(&DATA[0..(DATA.len() / 2)]).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC32 for {}: {}", name, e);
            return;
        }
    };

    let full_crc = match crc_handle.calculate(&DATA[(DATA.len() / 2)..]).await {
        Ok(crc) => crc,
        Err(e) => {
            error!("Error calculating CRC32 for {}: {}", name, e);
            return;
        }
    };

    if full_crc == algorithm.check {
        info!("Split calculation test passed (CRC = 0x{:04X})", full_crc);
    } else {
        error!(
            "RESUME TEST FAILED (Oneshot CRC = 0x{:04X}, Two shot CRC = 0x{:04X}, Expected CRC = 0x{:04X})",
            oneshot_crc, full_crc, algorithm.check
        );
    }
}
