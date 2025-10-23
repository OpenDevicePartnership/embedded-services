//! EC Internal Messages

use crate::ec_type::protocols::{acpi, debug, mctp::OdpCommandCode, mptf};

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug)]
pub enum CapabilitiesMessage {
    Events(u32),
    FwVersion(super::structure::Version),
    SecureState(u8),
    BootStatus(u8),
    FanMask(u8),
    BatteryMask(u8),
    TempMask(u16),
    KeyMask(u16),
    DebugMask(u16),
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeAlarmMessage {
    Events(u32),
    Capability(u32),
    Year(u16),
    Month(u8),
    Day(u8),
    Hour(u8),
    Minute(u8),
    Second(u8),
    Valid(u8),
    Daylight(u8),
    Res1(u8),
    Milli(u16),
    TimeZone(u16),
    Res2(u16),
    AlarmStatus(u32),
    AcTimeVal(u32),
    DcTimeVal(u32),
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BatteryMessage {
    Events(u32),
    Status(u32),
    LastFullCharge(u32),
    CycleCount(u32),
    State(u32),
    PresentRate(u32),
    RemainCap(u32),
    PresentVolt(u32),
    PsrState(u32),
    PsrMaxOut(u32),
    PsrMaxIn(u32),
    PeakLevel(u32),
    PeakPower(u32),
    SusLevel(u32),
    SusPower(u32),
    PeakThres(u32),
    SusThres(u32),
    TripThres(u32),
    BmcData(u32),
    BmdData(u32),
    BmdFlags(u32),
    BmdCount(u32),
    ChargeTime(u32),
    RunTime(u32),
    SampleTime(u32),
}

/// ACPI Message, compatible with comms system
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HostRequest<Command: Copy, Payload: Copy> {
    /// Command
    pub command: Command,
    /// Status code
    pub status: u8,
    /// Data payload
    pub payload: Payload,
}

/// Notification type to be sent to Host
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct NotificationMsg {
    /// Interrupt offset
    pub offset: u8,
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThermalMessage {
    Events(u32),
    CoolMode(u32),
    DbaLimit(u32),
    SonneLimit(u32),
    MaLimit(u32),
    Fan1OnTemp(u32),
    Fan1RampTemp(u32),
    Fan1MaxTemp(u32),
    Fan1CrtTemp(u32),
    Fan1HotTemp(u32),
    Fan1MaxRpm(u32),
    Fan1CurRpm(u32),
    Tmp1Val(u32),
    Tmp1Timeout(u32),
    Tmp1Low(u32),
    Tmp1High(u32),
}

/// Message type that services can send to communicate with the Host.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HostMsg<Command: Copy, Payload: Copy> {
    /// Notification without data. After receivng a notification,
    /// typically the host will request some data from the EC
    Notification(NotificationMsg),
    /// Response to Host request.
    Response(HostRequest<Command, Payload>),
}

/// ODP specific command code that can come in from the host.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OdpCommand {
    /// Battery commands
    Battery(acpi::BatteryCmd),
    /// Thermal commands
    Thermal(mptf::ThermalCmd),
    /// Debug commands
    Debug(debug::DebugCmd),
}

/// Standard Battery Service Model Number String Size
pub const STD_BIX_MODEL_SIZE: usize = 8;
/// Standard Battery Service Serial Number String Size
pub const STD_BIX_SERIAL_SIZE: usize = 8;
/// Standard Battery Service Battery Type String Size
pub const STD_BIX_BATTERY_SIZE: usize = 8;
/// Standard Battery Service OEM Info String Size
pub const STD_BIX_OEM_SIZE: usize = 8;
/// Standard Power Policy Service Model Number String Size
pub const STD_PIF_MODEL_SIZE: usize = 8;
/// Standard Power Policy Serial Number String Size
pub const STD_PIF_SERIAL_SIZE: usize = 8;
/// Standard Power Policy Service OEM Info String Size
pub const STD_PIF_OEM_SIZE: usize = 8;
/// Standard Debug Service Log Buffer Size
pub const STD_DEBUG_BUF_SIZE: usize = 128;

/// Standard ODP Host Payload
pub type StdHostPayload = crate::ec_type::protocols::mctp::Odp<
    STD_BIX_MODEL_SIZE,
    STD_BIX_SERIAL_SIZE,
    STD_BIX_BATTERY_SIZE,
    STD_BIX_OEM_SIZE,
    STD_PIF_MODEL_SIZE,
    STD_PIF_SERIAL_SIZE,
    STD_PIF_OEM_SIZE,
    STD_DEBUG_BUF_SIZE,
>;

/// Standard Host Request
pub type StdHostRequest = HostRequest<OdpCommand, StdHostPayload>;
/// Standard Host Message
pub type StdHostMsg = HostMsg<OdpCommand, StdHostPayload>;

impl From<OdpCommandCode> for OdpCommand {
    fn from(value: OdpCommandCode) -> Self {
        match value {
            OdpCommandCode::BatteryGetBixRequest | OdpCommandCode::BatteryGetBixResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBix)
            }
            OdpCommandCode::BatteryGetBstRequest | OdpCommandCode::BatteryGetBstResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBst)
            }
            OdpCommandCode::BatteryGetPsrRequest | OdpCommandCode::BatteryGetPsrResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetPsr)
            }
            OdpCommandCode::BatteryGetPifRequest | OdpCommandCode::BatteryGetPifResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetPif)
            }
            OdpCommandCode::BatteryGetBpsRequest | OdpCommandCode::BatteryGetBpsResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBps)
            }
            OdpCommandCode::BatterySetBtpRequest | OdpCommandCode::BatterySetBtpResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::SetBtp)
            }
            OdpCommandCode::BatterySetBptRequest | OdpCommandCode::BatterySetBptResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::SetBpt)
            }
            OdpCommandCode::BatteryGetBpcRequest | OdpCommandCode::BatteryGetBpcResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBpc)
            }
            OdpCommandCode::BatterySetBmcRequest | OdpCommandCode::BatterySetBmcResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::SetBmc)
            }
            OdpCommandCode::BatteryGetBmdRequest | OdpCommandCode::BatteryGetBmdResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBmd)
            }
            OdpCommandCode::BatteryGetBctRequest | OdpCommandCode::BatteryGetBctResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBct)
            }
            OdpCommandCode::BatteryGetBtmRequest | OdpCommandCode::BatteryGetBtmResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetBtm)
            }
            OdpCommandCode::BatterySetBmsRequest | OdpCommandCode::BatterySetBmsResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::SetBms)
            }
            OdpCommandCode::BatterySetBmaRequest | OdpCommandCode::BatterySetBmaResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::SetBma)
            }
            OdpCommandCode::BatteryGetStaRequest | OdpCommandCode::BatteryGetStaResponse => {
                OdpCommand::Battery(acpi::BatteryCmd::GetSta)
            }
            OdpCommandCode::ThermalGetTmpRequest | OdpCommandCode::ThermalGetTmpResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::GetTmp)
            }
            OdpCommandCode::ThermalSetThrsRequest | OdpCommandCode::ThermalSetThrsResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::SetThrs)
            }
            OdpCommandCode::ThermalGetThrsRequest | OdpCommandCode::ThermalGetThrsResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::GetThrs)
            }
            OdpCommandCode::ThermalSetScpRequest | OdpCommandCode::ThermalSetScpResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::SetScp)
            }
            OdpCommandCode::ThermalGetVarRequest | OdpCommandCode::ThermalGetVarResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::GetVar)
            }
            OdpCommandCode::ThermalSetVarRequest | OdpCommandCode::ThermalSetVarResponse => {
                OdpCommand::Thermal(mptf::ThermalCmd::SetVar)
            }
            OdpCommandCode::DebugGetMsgsRequest | OdpCommandCode::DebugGetMsgsResponse => {
                OdpCommand::Debug(debug::DebugCmd::GetMsgs)
            }
        }
    }
}

// TODO: Maybe map to Response instead?
impl From<OdpCommand> for OdpCommandCode {
    fn from(value: OdpCommand) -> Self {
        match value {
            OdpCommand::Battery(acpi::BatteryCmd::GetBix) => OdpCommandCode::BatteryGetBixRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBst) => OdpCommandCode::BatteryGetBstRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetPsr) => OdpCommandCode::BatteryGetPsrRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetPif) => OdpCommandCode::BatteryGetPifRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBps) => OdpCommandCode::BatteryGetBpsRequest,
            OdpCommand::Battery(acpi::BatteryCmd::SetBtp) => OdpCommandCode::BatterySetBtpRequest,
            OdpCommand::Battery(acpi::BatteryCmd::SetBpt) => OdpCommandCode::BatterySetBptRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBpc) => OdpCommandCode::BatteryGetBpcRequest,
            OdpCommand::Battery(acpi::BatteryCmd::SetBmc) => OdpCommandCode::BatterySetBmcRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBmd) => OdpCommandCode::BatteryGetBmdRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBct) => OdpCommandCode::BatteryGetBctRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetBtm) => OdpCommandCode::BatteryGetBtmRequest,
            OdpCommand::Battery(acpi::BatteryCmd::SetBms) => OdpCommandCode::BatterySetBmsRequest,
            OdpCommand::Battery(acpi::BatteryCmd::SetBma) => OdpCommandCode::BatterySetBmaRequest,
            OdpCommand::Battery(acpi::BatteryCmd::GetSta) => OdpCommandCode::BatteryGetStaRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::GetTmp) => OdpCommandCode::ThermalGetTmpRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::SetThrs) => OdpCommandCode::ThermalSetThrsRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::GetThrs) => OdpCommandCode::ThermalGetThrsRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::SetScp) => OdpCommandCode::ThermalSetScpRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::GetVar) => OdpCommandCode::ThermalGetVarRequest,
            OdpCommand::Thermal(mptf::ThermalCmd::SetVar) => OdpCommandCode::ThermalSetVarRequest,
            OdpCommand::Debug(debug::DebugCmd::GetMsgs) => OdpCommandCode::DebugGetMsgsRequest,
        }
    }
}
