//! Definitions for standard MPTF messages the generic Thermal service can expect
//! These will need to be fleshed out and ensure they meet required MPTF/ACPI specs and whatnot
//!
//! Transport services such as eSPI and SSH would then need to make sure they agree on this format
//! as they would then be responsible for converting raw data into these types and forwarding them
//! to thermal.
pub type TzId = u8;
pub type Dword = u32;
pub type DeciKelvin = Dword;

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub enum Request {
    // EC_THM_GET_TMP
    GetTmp(TzId),

    // EC_THM_GET_THRS
    GetThrs(TzId),
    // EC_THM_SET_THRS
    SetThrs(TzId, Dword, DeciKelvin, DeciKelvin),

    // EC_THM_SET_SCP
    SetScp(TzId, Dword, Dword, Dword),

    // EC_THM_GET_VAR(218246e7-baf6-45f1-aa13-07e4845256b8)
    GetCrtTemp,
    // EC_THM_SET_VAR(218246e7-baf6-45f1-aa13-07e4845256b8)
    SetCrtTemp(DeciKelvin),

    // EC_THM_GET_VAR(22dc52d2-fd0b-47ab-95b8-26552f9831a5)
    GetProcHotTemp,
    // EC_THM_SET_VAR(22dc52d2-fd0b-47ab-95b8-26552f9831a5)
    SetProcHotTemp(DeciKelvin),

    // EC_THM_GET_VAR(23b4a025-cdfd-4af9-a411-37a24c574615)
    GetProfileType,
    // EC_THM_SET_VAR(23b4a025-cdfd-4af9-a411-37a24c574615)
    SetProfileType(Dword),

    // EC_THM_GET_VAR(ba17b567-c368-48d5-bc6f-a312a41583c1)
    GetFanOnTemp,
    // EC_THM_SET_VAR(ba17b567-c368-48d5-bc6f-a312a41583c1)
    SetFanOnTemp(DeciKelvin),

    // EC_THM_GET_VAR(3a62688c-d95b-4d2d-bacc-90d7a5816bcd)
    GetFanRampTemp,
    // EC_THM_SET_VAR(3a62688c-d95b-4d2d-bacc-90d7a5816bcd)
    SetFanRampTemp(DeciKelvin),

    // EC_THM_GET_VAR(dcb758b1-f0fd-4ec7-b2c0-ef1e2a547b76)
    GetFanMaxTemp,
    // EC_THM_SET_VAR(dcb758b1-f0fd-4ec7-b2c0-ef1e2a547b76)
    SetFanMaxTemp(DeciKelvin),

    // EC_THM_GET_VAR(db261c77-934b-45e2-9742-256c62badb7a)
    GetFanMinRpm,

    // EC_THM_GET_VAR(5cf839df-8be7-42b9-9ac5-3403ca2c8a6a)
    GetFanMaxRpm,

    // EC_THM_GET_VAR(adf95492-0776-4ffc-84f3-b6c8b5269683)
    GetFanCurrentRpm,

    // EC_THM_GET_VAR()
    GetFanMinDba,

    // EC_THM_GET_VAR()
    GetFanMaxDba,

    // EC_THM_GET_VAR()
    GetFanCurrentDba,

    // EC_THM_GET_VAR()
    GetFanMinSones,

    // EC_THM_GET_VAR()
    GetFanMaxSones,

    // EC_THM_GET_VAR()
    GetFanCurrentSones,
}

#[derive(Debug, Clone, Copy)]
pub enum Response {
    // Standard command
    GetTmp(DeciKelvin),
    GetThrs(Dword, DeciKelvin, DeciKelvin),
    SetThrs,
    SetScp,

    // DWORD Variable - Thermal
    GetCrtTemp(DeciKelvin),
    SetCrtTemp,
    GetProcHotTemp(DeciKelvin),
    SetProcHotTemp,
    GetProfileType(DeciKelvin),
    SetProfileType,

    // DWORD Variable - Fan
    GetFanOnTemp(DeciKelvin),
    SetFanOnTemp,
    GetFanRampTemp(DeciKelvin),
    SetFanRampTemp,
    GetFanMaxTemp(DeciKelvin),
    SetFanMaxTemp,
    GetFanMinRpm(Dword),
    GetFanMaxRpm(Dword),
    GetFanCurrentRpm(Dword),

    // DWORD Variable - Fan Optional
    GetFanMinDba(Dword),
    GetFanMaxDba(Dword),
    GetFanCurrentDba(Dword),
    GetFanMinSones(Dword),
    GetFanMaxSones(Dword),
    GetFanCurrentSones(Dword),
}

#[derive(Debug, Clone, Copy)]
pub enum Notify {
    Threshold,
    Critical,
    ProcHot,
}
