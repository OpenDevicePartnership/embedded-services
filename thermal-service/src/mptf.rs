//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed

/// MPTF Standard UIDs which the default service understands
// TODO: Put these UIDs into the actual correct format according to spec
pub mod uid {
    use super::Uid;
    pub const CRT_TEMP: Uid = 0x218246e7baf645f1aa1307e4845256b8;
    pub const PROC_HOT_TEMP: Uid = 0x22dc52d2fd0b47ab95b826552f9831a5;
    pub const PROFILE_TYPE: Uid = 0x23b4a025cdfd4af9a41137a24c574615;
    pub const FAN_ON_TEMP: Uid = 0xba17b567c36848d5bc6fa312a41583c1;
    pub const FAN_RAMP_TEMP: Uid = 0x3a62688cd95b4d2dbacc90d7a5816bcd;
    pub const FAN_MAX_TEMP: Uid = 0xdcb758b1f0fd4ec7b2c0ef1e2a547b76;
    pub const FAN_MIN_RPM: Uid = 0xdb261c77934b45e29742256c62badb7a;
    pub const FAN_MAX_RPM: Uid = 0x5cf839df8be742b99ac53403ca2c8a6a;
    pub const FAN_CURRENT_RPM: Uid = 0xadf9549207764ffc84f3b6c8b5269683;
}

/// Standard 32-bit DWORD
pub type Dword = u32;

/// Thermalzone ID
pub type TzId = u8;

/// Time in miliseconds
pub type Miliseconds = Dword;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = Dword;

/// UID underlying representation
pub type Uid = u128;

/// Helper MPTF Response type
pub type Response = Result<ResponseData, Error>;

/// Error codes expected by MPTF
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    InvalidParameter,
    UnsupportedRevision,
    HardwareError,
}

impl From<Error> for u8 {
    fn from(value: Error) -> Self {
        match value {
            Error::InvalidParameter => 1,
            Error::UnsupportedRevision => 2,
            Error::HardwareError => 3,
        }
    }
}

/// Standard MPTF requests expected by the thermal subsystem
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    // EC_THM_GET_TMP
    GetTmp(TzId),
    // EC_THM_GET_THRS
    GetThrs(TzId),
    // EC_THM_SET_THRS
    SetThrs(TzId, Miliseconds, DeciKelvin, DeciKelvin),
    // EC_THM_SET_SCP
    SetScp(TzId, Dword, Dword, Dword),
    // EC_THM_GET_VAR
    GetVar(u8, Uid),
    // EC_THM_SET_VAR
    SetVar(u8, Uid, Dword),
}

/// Data returned by thermal subsystem in response to MPTF requests  
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    // Standard command
    GetTmp(DeciKelvin),
    GetThrs(Miliseconds, DeciKelvin, DeciKelvin),
    SetThrs,
    SetScp,
    GetVar(u8, Uid, Dword),
    SetVar,
}

/// Notifications to Host
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Notify {
    Warn,
    Critical,
    ProcHot,
}
