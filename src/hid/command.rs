//! Date structures and code for handling HID commands

use core::borrow::Borrow;

use crate::buffer::SharedRef;
/// HID report ID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ReportId(pub u8);

/// HID report types
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReportType {
    /// Input report
    Input,
    /// Output report
    Output,
    /// Feature report
    Feature,
}

const FEATURE_MASK: u16 = 0x30;
const FEATURE_SHIFT: u16 = 4;

impl TryFrom<u16> for ReportType {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match (value & FEATURE_MASK) >> FEATURE_SHIFT {
            0x01 => Ok(ReportType::Input),
            0x02 => Ok(ReportType::Output),
            0x03 => Ok(ReportType::Feature),
            _ => Err(()),
        }
    }
}

impl Into<u16> for ReportType {
    fn into(self) -> u16 {
        match self {
            ReportType::Input => 0x01 << FEATURE_SHIFT,
            ReportType::Output => 0x02 << FEATURE_SHIFT,
            ReportType::Feature => 0x03 << FEATURE_SHIFT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Power state
pub enum PowerState {
    /// On
    On,
    /// Sleep
    Sleep,
}

const POWER_STATE_MASK: u16 = 0x3;
impl TryFrom<u16> for PowerState {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value & POWER_STATE_MASK {
            0x0 => Ok(PowerState::On),
            0x1 => Ok(PowerState::Sleep),
            _ => Err(()),
        }
    }
}

impl Into<u16> for PowerState {
    fn into(self) -> u16 {
        match self {
            PowerState::On => 0x0,
            PowerState::Sleep => 0x1,
        }
    }
}

/// Report frequency, see spec for more details
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum ReportFreq {
    Infinite,
    Msecs(u16),
}

impl TryFrom<u16> for ReportFreq {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(ReportFreq::Infinite),
            _ => Ok(ReportFreq::Msecs(value)),
        }
    }
}

impl Into<u16> for ReportFreq {
    fn into(self) -> u16 {
        match self {
            ReportFreq::Infinite => 0x0,
            ReportFreq::Msecs(value) => value,
        }
    }
}

/// HID device protocol, see spec for more details
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum Protocol {
    Boot,
    Report,
}

impl TryFrom<u16> for Protocol {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(Protocol::Boot),
            0x1 => Ok(Protocol::Report),
            _ => Err(()),
        }
    }
}

impl Into<u16> for Protocol {
    fn into(self) -> u16 {
        match self {
            Protocol::Boot => 0x0,
            Protocol::Report => 0x1,
        }
    }
}

/// Command opcodes, see spec for more details
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum CommandOpcode {
    Reset,
    GetReport,
    SetReport,
    GetIdle,
    SetIdle,
    GetProtocol,
    SetProtocol,
    SetPower,
    Vendor,
}

const OPCODE_MASK: u16 = 0xf00;
const OPCODE_SHIFT: u16 = 8;

impl TryFrom<u16> for CommandOpcode {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match (value & OPCODE_MASK) >> OPCODE_SHIFT {
            0x01 => Ok(CommandOpcode::Reset),
            0x02 => Ok(CommandOpcode::GetReport),
            0x03 => Ok(CommandOpcode::SetReport),
            0x04 => Ok(CommandOpcode::GetIdle),
            0x05 => Ok(CommandOpcode::SetIdle),
            0x06 => Ok(CommandOpcode::GetProtocol),
            0x07 => Ok(CommandOpcode::SetProtocol),
            0x08 => Ok(CommandOpcode::SetPower),
            0x0e => Ok(CommandOpcode::Vendor),
            _ => Err(()),
        }
    }
}
impl Into<u16> for CommandOpcode {
    fn into(self) -> u16 {
        match self {
            CommandOpcode::Reset => 0x01 << OPCODE_SHIFT,
            CommandOpcode::GetReport => 0x02 << OPCODE_SHIFT,
            CommandOpcode::SetReport => 0x03 << OPCODE_SHIFT,
            CommandOpcode::GetIdle => 0x04 << OPCODE_SHIFT,
            CommandOpcode::SetIdle => 0x05 << OPCODE_SHIFT,
            CommandOpcode::GetProtocol => 0x06 << OPCODE_SHIFT,
            CommandOpcode::SetProtocol => 0x07 << OPCODE_SHIFT,
            CommandOpcode::SetPower => 0x08 << OPCODE_SHIFT,
            CommandOpcode::Vendor => 0x0e << OPCODE_SHIFT,
        }
    }
}

impl CommandOpcode {
    /// Return true if the command has data to read from the host
    pub fn requires_host_data(&self) -> bool {
        match self {
            CommandOpcode::SetReport | CommandOpcode::SetIdle | CommandOpcode::Vendor => true,
            _ => false,
        }
    }

    /// Return true if the command requires a report ID
    pub fn requires_report_id(&self) -> bool {
        match self {
            CommandOpcode::GetReport | CommandOpcode::SetReport | CommandOpcode::GetIdle | CommandOpcode::SetIdle => {
                true
            }
            _ => false,
        }
    }

    /// Return true if the command has a response read from the data register
    pub fn has_response(&self) -> bool {
        match self {
            CommandOpcode::GetReport | CommandOpcode::GetIdle | CommandOpcode::GetProtocol => true,
            _ => false,
        }
    }
}

/// Host to device commands, see spec for more details
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum Command<'a> {
    Reset,
    GetReport(ReportType, ReportId),
    SetReport(ReportType, ReportId, SharedRef<'a>),
    GetIdle(ReportId),
    SetIdle(ReportId, ReportFreq),
    GetProtocol,
    SetProtocol(Protocol),
    SetPower(PowerState),
    Vendor,
}

/// Device command response, GetReport uses the standard report responses
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommandResponse {
    /// Get idle response
    GetIdle(ReportFreq),
    /// Get protocol response
    GetProtocol(Protocol),
    /// Vendor specific response
    Vendor,
}

/// Command creation errors
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommandError {
    /// Command requires a report ID
    RequiresReportId,
    /// Command requires data
    RequiresData,
    /// Invalid data provided
    InvalidData,
    /// Invalid report type for command
    InvalidReportType,
    /// Invalid report frequency
    InvalidReportFreq,
}

/// Value for extended report ID
pub const EXTENDED_REPORT_ID: u8 = 0xf;
const REPORT_ID_MASK: u16 = 0xf;

impl<'a> Command<'a> {
    /// Get report ID from data
    pub fn report_id(data: u16) -> ReportId {
        ReportId((data & REPORT_ID_MASK) as u8)
    }

    /// Check if the data has extended report ID
    pub fn has_extended_report_id(data: u16) -> bool {
        Self::report_id(data).0 == EXTENDED_REPORT_ID
    }

    /// Creates a new command with validation
    pub fn new(
        cmd: u16,
        opcode: CommandOpcode,
        report_type: Option<ReportType>,
        report_id: Option<ReportId>,
        data: Option<SharedRef<'static>>,
    ) -> Result<Self, CommandError> {
        if opcode.requires_report_id() && report_id.is_none() {
            return Err(CommandError::RequiresReportId);
        }

        if opcode.requires_host_data() && data.is_none() {
            // Vendor defined commands might or might not have data with them
            if opcode != CommandOpcode::Vendor {
                return Err(CommandError::RequiresData);
            }
        }

        let report_type = report_type.ok_or_else(|| CommandError::InvalidReportType);
        let command = match opcode {
            CommandOpcode::Reset => Command::Reset,
            CommandOpcode::GetReport => {
                if report_type? == ReportType::Input || report_type? == ReportType::Feature {
                    Command::GetReport(report_type?, report_id.unwrap())
                } else {
                    return Err(CommandError::InvalidReportType);
                }
            }
            CommandOpcode::SetReport => {
                if report_type? == ReportType::Output || report_type? == ReportType::Feature {
                    Command::SetReport(report_type?, report_id.unwrap(), data.unwrap())
                } else {
                    return Err(CommandError::InvalidReportType);
                }
            }
            CommandOpcode::GetIdle => Command::GetIdle(report_id.unwrap()),
            CommandOpcode::SetIdle => Command::SetIdle(
                report_id.unwrap(),
                cmd.try_into().map_err(|_| CommandError::InvalidReportFreq)?,
            ),
            CommandOpcode::GetProtocol => Command::GetProtocol,
            CommandOpcode::SetProtocol => Command::SetProtocol(cmd.try_into().map_err(|_| CommandError::InvalidData)?),
            CommandOpcode::SetPower => Command::SetPower(cmd.try_into().map_err(|_| CommandError::InvalidData)?),
            CommandOpcode::Vendor => Command::Vendor,
        };

        Ok(command)
    }

    /// Writes opcode, report feature, and report ID into a buffer
    fn write_report_info(
        opcode: CommandOpcode,
        report_type: Option<ReportType>,
        report_id: ReportId,
        buffer: &mut [u8],
    ) -> usize {
        let opcode_value: u16 = opcode.into();
        let type_value: u16 = report_type.map_or(0, Into::into);
        let value = report_id.0 as u16 | type_value | opcode_value;

        assert!(buffer.len() >= 2);
        buffer[0..2].copy_from_slice(&value.to_le_bytes());
        if report_id.0 == EXTENDED_REPORT_ID {
            assert!(buffer.len() >= 3);
            buffer[2] = report_id.0;
            3
        } else {
            2
        }
    }

    /// Serialize the command to bytes
    /// Returns a slice since the number of bytes can vary
    pub fn write_bytes(&self, buffer: &mut [u8]) -> usize {
        match self {
            Command::Reset => {
                let value: u16 = CommandOpcode::Reset.into();
                buffer[0..2].copy_from_slice(&value.to_le_bytes());
                2
            }
            Command::GetReport(report_type, report_id) => {
                Self::write_report_info(CommandOpcode::GetReport, Some(*report_type), *report_id, buffer)
            }
            Command::SetReport(report_type, repord_id, data) => {
                let borrow = data.borrow();
                let data: &[u8] = borrow.borrow();

                let len = Self::write_report_info(CommandOpcode::SetReport, Some(*report_type), *repord_id, buffer);
                buffer[len..data.len()].copy_from_slice(data);
                data.len() + len
            }
            Command::GetIdle(report_id) => Self::write_report_info(CommandOpcode::GetIdle, None, *report_id, buffer),
            Command::SetIdle(report_id, freq) => {
                let len = Self::write_report_info(CommandOpcode::SetIdle, None, *report_id, buffer);
                buffer[len..len + 2].copy_from_slice(&Into::<u16>::into(*freq).to_le_bytes());
                len + 2
            }
            Command::GetProtocol => {
                let value: u16 = CommandOpcode::GetProtocol.into();
                buffer[0..2].copy_from_slice(&value.to_le_bytes());
                2
            }
            Command::SetProtocol(protocol) => {
                let value: u16 = CommandOpcode::SetProtocol.into();
                let protocol: u16 = (*protocol).into();
                buffer[0..2].copy_from_slice(&value.to_le_bytes());
                buffer[2..4].copy_from_slice(&protocol.to_le_bytes());
                4
            }
            Command::SetPower(state) => {
                let opcode: u16 = CommandOpcode::SetPower.into();
                let state: u16 = (*state).into();
                let value = opcode | state;
                buffer[0..2].copy_from_slice(&value.to_le_bytes());
                2
            }
            _ => 0,
        }
    }
}
