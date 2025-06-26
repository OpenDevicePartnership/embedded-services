use embassy_imxrt::crc::{Config, Polynomial};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{Duration, WithTimeout};
use embedded_services::GlobalRawMutex;

use crate::embedded_crc::EmbeddedCrcError;

// Initialize static CRC IMXRT object to access hardware registers
// Locks register access when in use for each calculation
static CRC: OnceLock<Mutex<GlobalRawMutex, embassy_imxrt::crc::Crc<'static>>> = OnceLock::new();

pub const CRC_CCITT_POLY: u16 = 0x1021;
pub const CRC_CRC16_POLY: u16 = 0x8005;
pub const CRC_CRC32_POLY: u32 = 0x04C11DB7;
pub const CRC_CCITT_POLY_U32: u32 = 0x1021;
pub const CRC_CRC16_POLY_U32: u32 = 0x8005;

// Must be called once to initialize hardware resources, then EmbeddedCrc object can run with hardware-agnostic signatures
pub fn crc_engine_init(crc: embassy_imxrt::crc::Crc<'static>) {
    CRC.get_or_init(|| Mutex::new(crc));
}

pub(crate) async fn crc_calculate_u32(
    init: u32,
    algorithm: &'static crc::Algorithm<u32>,
    bytes: &[u8],
) -> Result<u32, EmbeddedCrcError> {
    // Validate and convert algorithm to config
    // Only three polynomials are supported in IMXRT CRC accelerator
    // Width 16
    //   CRC-CCITT: 0x1021 = x16 + x12 + x5 + 1
    //   CRC-16:    0x8005 = x16 + x15 + x2 + 1
    // Width 32
    //   CRC-32:    0x04C11DB7 =  x32 + x26 + x23 + x22 + x16 + x12 + x11 + x10 + x8 + x7 + x5 + x4 + x2 + x + 1
    // u32 implementation supports 16-bit width algorithms with u32 data type
    let polynomial: Polynomial = match algorithm.width {
        16 => {
            if algorithm.xorout != 0 && algorithm.xorout != 0xFFFF {
                return Err(EmbeddedCrcError::CrcErrorXorOut);
            }
            match algorithm.poly {
                CRC_CCITT_POLY_U32 => Polynomial::CrcCcitt,
                CRC_CRC16_POLY_U32 => Polynomial::Crc16,
                _ => {
                    return Err(EmbeddedCrcError::CrcErrorPolynomial);
                }
            }
        }
        32 => {
            if algorithm.xorout != 0 && algorithm.xorout != 0xFFFFFFFF {
                return Err(EmbeddedCrcError::CrcErrorXorOut);
            }
            if algorithm.poly == CRC_CRC32_POLY {
                Polynomial::Crc32
            } else {
                return Err(EmbeddedCrcError::CrcErrorPolynomial);
            }
        }
        _ => {
            return Err(EmbeddedCrcError::CrcErrorWidth);
        }
    };

    let mut crc = match CRC.get().await.lock().with_timeout(Duration::from_millis(500)).await {
        Ok(crc) => crc,
        Err(_e) => {
            return Err(EmbeddedCrcError::CrcErrorMutexGet);
        }
    };

    crc.reconfigure(Config {
        polynomial,
        reverse_in: algorithm.refin,
        reverse_out: algorithm.refout,
        complement_in: false,
        complement_out: algorithm.xorout == 0xFFFFFFFF || algorithm.xorout == 0xFFFF,
        seed: init,
    });

    Ok(crc.feed_bytes(bytes))
}

pub(crate) async fn crc_calculate_u16(
    init: u16,
    algorithm: &'static crc::Algorithm<u16>,
    bytes: &[u8],
) -> Result<u16, EmbeddedCrcError> {
    // Validate and convert algorithm to config
    // Only two 16-bit polynomials are supported in IMXRT CRC accelerator
    // Width 16
    //   CRC-CCITT: 0x1021 = x16 + x12 + x5 + 1
    //   CRC-16:    0x8005 = x16 + x15 + x2 + 1
    let polynomial: Polynomial = if algorithm.width == 16 {
        if algorithm.xorout != 0 && algorithm.xorout != 0xFFFF {
            return Err(EmbeddedCrcError::CrcErrorXorOut);
        }

        match algorithm.poly {
            CRC_CCITT_POLY => Polynomial::CrcCcitt,
            CRC_CRC16_POLY => Polynomial::Crc16,
            _ => {
                return Err(EmbeddedCrcError::CrcErrorPolynomial);
            }
        }
    } else {
        return Err(EmbeddedCrcError::CrcErrorWidth);
    };

    let mut crc = match CRC.get().await.lock().with_timeout(Duration::from_millis(500)).await {
        Ok(crc) => crc,
        Err(_e) => {
            return Err(EmbeddedCrcError::CrcErrorMutexGet);
        }
    };
    crc.reconfigure(Config {
        polynomial,
        reverse_in: algorithm.refin,
        reverse_out: algorithm.refout,
        complement_in: false,
        complement_out: algorithm.xorout == 0xFFFF,
        seed: init as u32,
    });

    Ok(crc.feed_bytes(bytes) as u16)
}
