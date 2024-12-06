//! Ucsi types, see spec at https://www.usb.org/document-library/usb-type-cr-connector-system-software-interface-ucsi-specification
#![allow(missing_docs)]

use bitfield::bitfield;

use crate::type_c::Error;

pub mod lpm;
pub mod ppm;

/// Ucsi opcodes, see spec for more detail
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UsciOpcode {
    PpmReset = 0x01,
    Cancel,
    ConnectorReset,
    AckCcCi,
    SetNotificationEnable,
    GetCapability,
    GetConnectorCapability,
    SetCcom,
    SetUor,
    SetPdm,
    SetPdr,
    GetAlternateModes,
    GetCamSupported,
    GetCurrentCam,
    SetNewCam,
    GetPdos,
    GetCableProperty,
    GetConnectorStatus,
    GetErrorStatus,
    SetPowerLevel,
    GetPdMessage,
    GetAttentionVdo,
    GetCamCs = 0x18,
    LpmFwUpdateRequest,
    SecurityRequest,
    SetRetimerMode,
    SetSinkPath,
    SetPdos,
    ReadPowerLevel,
    ChunkingSupport,
    SetUsb = 0x21,
    GetLpmPpmInfo,
}

bitfield! {
    /// Command status and connect change indicator, see spec for more details
    #[derive(Copy, Clone)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct Cci(u32);
    impl Debug;
    pub eom, set_eom: 0, 0;
    pub port, set_port: 1, 7;
    pub data_len, set_data_len: 8, 15;
    pub vdm, set_vdm: 16, 16;
    pub reserved, _: 17, 22;
    pub security_req, set_security_req: 23, 23;
    pub fw_update_req, set_fw_update_req: 24, 24;
    pub not_supported, set_not_supported: 25, 25;
    pub cancel_complete, set_cancel_complete: 26, 26;
    pub reset_complete, set_reset_complete: 27, 27;
    pub busy, set_busy: 28, 28;
    pub ack_command, set_ack_command: 29, 29;
    pub error, set_error: 30, 30;
    pub cmd_complete, set_cmd_complete: 31, 31;
}

/// UCSI commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    PpmCommand(ppm::Command),
    LpmCommand(lpm::Command),
}

/// UCSI command responses
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    PpmResponse(Result<ppm::ResponseData, Error>),
    LpmResponse(Result<lpm::ResponseData, Error>),
}
