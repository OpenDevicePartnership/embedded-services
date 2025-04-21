//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed
use core::future::Future;

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
    GetVar(Uid),
    // EC_THM_SET_VAR
    SetVar(Uid, Dword),
}

/// Data returned by thermal subsystem in response to MPTF requests  
#[derive(Debug, Clone, Copy)]
pub enum ResponseData {
    // Standard command
    GetTmp(DeciKelvin),
    GetThrs(Miliseconds, DeciKelvin, DeciKelvin),
    SetThrs,
    SetScp,
    GetVar(Uid, Dword),
    SetVar,
}

/// Notifications to Host
#[derive(Debug, Clone, Copy)]
pub enum Notify {
    Threshold,
    Critical,
    ProcHot,
}

/// Trait capturing the standard MPTF input/output interface for thermal subsystem
pub trait Handler {
    fn get_tmp(&self, tzid: TzId) -> impl Future<Output = Response>;
    fn get_thrs(&self, tzid: TzId) -> impl Future<Output = Response>;
    fn set_thrs(&self, tzid: TzId, timeout: Dword, low: DeciKelvin, high: DeciKelvin)
        -> impl Future<Output = Response>;
    fn set_scp(&self, tzid: TzId, mode: Dword, acoustic_lim: Dword, power_lim: Dword)
        -> impl Future<Output = Response>;
    fn get_var(&self, uid: Uid) -> impl Future<Output = Response>;
    fn set_var(&self, uid: Uid, value: Dword) -> impl Future<Output = Response>;
}
