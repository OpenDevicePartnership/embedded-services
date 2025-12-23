//! Data structures and code for handling HID commands
use core::borrow::Borrow;

use super::{Error, ReportId};
use crate::{buffer::SharedRef, hid::InvalidSizeError};

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
/// Size of a command with an extended report ID
const EXTENDED_REPORT_CMD_LEN: usize = 3;
/// Size of a command with a standard report ID
const STANDARD_REPORT_CMD_LEN: usize = 2;
/// Size of a command with no additional data
const BASIC_CMD_LEN: usize = 2;
/// Size of a register value
const REGISTER_LEN: usize = 2;
/// Size of a 16-bit value prefixed with a length
const LENGTH_VALUE_LEN: usize = 4;
/// Standard 16-bit value length
const VALUE_LEN: usize = 2;

impl TryFrom<u16> for ReportType {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match (value & FEATURE_MASK) >> FEATURE_SHIFT {
            0x01 => Ok(ReportType::Input),
            0x02 => Ok(ReportType::Output),
            0x03 => Ok(ReportType::Feature),
            _ => Err(Error::Serialize),
        }
    }
}

impl From<ReportType> for u16 {
    fn from(value: ReportType) -> Self {
        match value {
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
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value & POWER_STATE_MASK {
            0x0 => Ok(PowerState::On),
            0x1 => Ok(PowerState::Sleep),
            _ => Err(Error::Serialize),
        }
    }
}

impl From<PowerState> for u16 {
    fn from(value: PowerState) -> Self {
        match value {
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
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(ReportFreq::Infinite),
            _ => Ok(ReportFreq::Msecs(value)),
        }
    }
}

impl From<ReportFreq> for u16 {
    fn from(value: ReportFreq) -> Self {
        match value {
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
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(Protocol::Boot),
            0x1 => Ok(Protocol::Report),
            _ => Err(Error::Serialize),
        }
    }
}

impl From<Protocol> for u16 {
    fn from(value: Protocol) -> Self {
        match value {
            Protocol::Boot => 0x0,
            Protocol::Report => 0x1,
        }
    }
}

/// Command opcodes, see spec for more details
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum Opcode {
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

impl TryFrom<u16> for Opcode {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match (value & OPCODE_MASK) >> OPCODE_SHIFT {
            0x01 => Ok(Opcode::Reset),
            0x02 => Ok(Opcode::GetReport),
            0x03 => Ok(Opcode::SetReport),
            0x04 => Ok(Opcode::GetIdle),
            0x05 => Ok(Opcode::SetIdle),
            0x06 => Ok(Opcode::GetProtocol),
            0x07 => Ok(Opcode::SetProtocol),
            0x08 => Ok(Opcode::SetPower),
            0x0e => Ok(Opcode::Vendor),
            _ => Err(Error::Serialize),
        }
    }
}

impl From<Opcode> for u16 {
    fn from(value: Opcode) -> Self {
        match value {
            Opcode::Reset => 0x01 << OPCODE_SHIFT,
            Opcode::GetReport => 0x02 << OPCODE_SHIFT,
            Opcode::SetReport => 0x03 << OPCODE_SHIFT,
            Opcode::GetIdle => 0x04 << OPCODE_SHIFT,
            Opcode::SetIdle => 0x05 << OPCODE_SHIFT,
            Opcode::GetProtocol => 0x06 << OPCODE_SHIFT,
            Opcode::SetProtocol => 0x07 << OPCODE_SHIFT,
            Opcode::SetPower => 0x08 << OPCODE_SHIFT,
            Opcode::Vendor => 0x0e << OPCODE_SHIFT,
        }
    }
}

impl Opcode {
    /// Return true if the command has data to read from the host
    pub fn requires_host_data(&self) -> bool {
        matches!(self, Opcode::SetReport | Opcode::SetIdle | Opcode::Vendor)
    }

    /// Return true if the command requires a report ID
    pub fn requires_report_id(&self) -> bool {
        matches!(
            self,
            Opcode::GetReport | Opcode::SetReport | Opcode::GetIdle | Opcode::SetIdle
        )
    }

    /// Return true if the command has a response read from the data register
    pub fn has_response(&self) -> bool {
        matches!(self, Opcode::GetReport | Opcode::GetIdle | Opcode::GetProtocol)
    }
}

/// Host to device commands, see spec for more details
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum Command<'a> {
    Reset,
    GetReport(ReportType, ReportId),
    SetReport(ReportType, ReportId, SharedRef<'a, u8>),
    GetIdle(ReportId),
    SetIdle(ReportId, ReportFreq),
    GetProtocol,
    SetProtocol(Protocol),
    SetPower(PowerState),
    Vendor,
}

impl From<Command<'_>> for Opcode {
    fn from(value: Command<'_>) -> Self {
        match value {
            Command::Reset => Opcode::Reset,
            Command::GetReport(_, _) => Opcode::GetReport,
            Command::SetReport(_, _, _) => Opcode::SetReport,
            Command::GetIdle(_) => Opcode::GetIdle,
            Command::SetIdle(_, _) => Opcode::SetIdle,
            Command::GetProtocol => Opcode::GetProtocol,
            Command::SetProtocol(_) => Opcode::SetProtocol,
            Command::SetPower(_) => Opcode::SetPower,
            Command::Vendor => Opcode::Vendor,
        }
    }
}

impl From<&Command<'_>> for Opcode {
    fn from(value: &Command<'_>) -> Self {
        value.clone().into()
    }
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

/// Value for extended report ID
pub const EXTENDED_REPORT_ID: u8 = 0xf;
const REPORT_ID_MASK: u16 = 0xf;

impl ReportId {
    /// Get report ID from command
    pub fn from_command(cmd: u16) -> ReportId {
        ReportId((cmd & REPORT_ID_MASK) as u8)
    }

    /// Check if the command has extended report ID
    pub fn has_extended_report_id(cmd: u16) -> bool {
        Self::from_command(cmd).0 == EXTENDED_REPORT_ID
    }
}

impl<'a> Command<'a> {
    /// Creates a new command with validation
    pub fn new(
        cmd: u16,
        opcode: Opcode,
        report_type: Option<ReportType>,
        report_id: Option<ReportId>,
        data: Option<SharedRef<'a, u8>>,
    ) -> Result<Self, Error> {
        if opcode.requires_report_id() && report_id.is_none() {
            return Err(Error::RequiresReportId);
        }

        if opcode.requires_host_data() && data.is_none() {
            // Vendor defined commands might or might not have data with them
            if opcode != Opcode::Vendor {
                return Err(Error::RequiresData);
            }
        }

        let report_type = report_type.ok_or(Error::InvalidReportType);
        let command = match opcode {
            Opcode::Reset => Command::Reset,
            Opcode::GetReport => {
                if report_type? == ReportType::Input || report_type? == ReportType::Feature {
                    Command::GetReport(report_type?, report_id.ok_or(Error::RequiresReportId)?)
                } else {
                    return Err(Error::InvalidReportType);
                }
            }
            Opcode::SetReport => {
                if report_type? == ReportType::Output || report_type? == ReportType::Feature {
                    Command::SetReport(
                        report_type?,
                        report_id.ok_or(Error::RequiresReportId)?,
                        data.ok_or(Error::RequiresData)?,
                    )
                } else {
                    return Err(Error::InvalidReportType);
                }
            }
            Opcode::GetIdle => Command::GetIdle(report_id.ok_or(Error::RequiresReportId)?),
            Opcode::SetIdle => Command::SetIdle(
                report_id.ok_or(Error::RequiresReportId)?,
                cmd.try_into().map_err(|_| Error::InvalidReportFreq)?,
            ),
            Opcode::GetProtocol => Command::GetProtocol,
            Opcode::SetProtocol => Command::SetProtocol(cmd.try_into().map_err(|_| Error::InvalidData)?),
            Opcode::SetPower => Command::SetPower(cmd.try_into().map_err(|_| Error::InvalidData)?),
            Opcode::Vendor => Command::Vendor,
        };

        Ok(command)
    }

    /// Encodes common values for a command with a report ID into a slice
    /// Returns the number of bytes written and the remaining buffer
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    fn encode_common(
        buf: &mut [u8],
        opcode: Opcode,
        report_type: Option<ReportType>,
        report_id: ReportId,
    ) -> Result<(usize, &mut [u8]), Error> {
        let mut val: u16 = opcode.into();

        val |= report_type.map_or(0, |x| x.into());

        if report_id.0 >= EXTENDED_REPORT_ID {
            if buf.len() < EXTENDED_REPORT_CMD_LEN {
                return Err(Error::InvalidSize(InvalidSizeError {
                    expected: EXTENDED_REPORT_CMD_LEN,
                    actual: buf.len(),
                }));
            }

            val |= EXTENDED_REPORT_ID as u16;

            // Copy standard data encoding the presence of an extended report ID
            buf[0..STANDARD_REPORT_CMD_LEN].copy_from_slice(&val.to_le_bytes());
            // Append extended report ID
            buf[STANDARD_REPORT_CMD_LEN] = report_id.0;

            Ok((EXTENDED_REPORT_CMD_LEN, &mut buf[EXTENDED_REPORT_CMD_LEN..]))
        } else {
            if buf.len() < STANDARD_REPORT_CMD_LEN {
                return Err(Error::InvalidSize(InvalidSizeError {
                    expected: STANDARD_REPORT_CMD_LEN,
                    actual: buf.len(),
                }));
            }

            val |= report_id.0 as u16;

            buf[0..STANDARD_REPORT_CMD_LEN].copy_from_slice(&val.to_le_bytes());
            Ok((STANDARD_REPORT_CMD_LEN, &mut buf[STANDARD_REPORT_CMD_LEN..]))
        }
    }

    /// Encodes an operation with no report ID or additional data into a slice
    /// Returns the number of bytes written and the remaining buffer
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    fn encode_basic_op(buf: &mut [u8], opcode: Opcode) -> Result<(usize, &mut [u8]), Error> {
        if buf.len() < BASIC_CMD_LEN {
            return Err(Error::InvalidSize(InvalidSizeError {
                expected: BASIC_CMD_LEN,
                actual: buf.len(),
            }));
        }

        buf[0..BASIC_CMD_LEN].copy_from_slice(&<Opcode as Into<u16>>::into(opcode).to_le_bytes());
        Ok((BASIC_CMD_LEN, &mut buf[BASIC_CMD_LEN..]))
    }

    /// Encodes a register address into a slice
    /// Returns the number of bytes written and the remaining buffer
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    fn encode_register(buf: &mut [u8], reg: Option<u16>) -> Result<(usize, &mut [u8]), Error> {
        if let Some(reg) = reg {
            if buf.len() < REGISTER_LEN {
                return Err(Error::InvalidSize(InvalidSizeError {
                    expected: REGISTER_LEN,
                    actual: buf.len(),
                }));
            }
            buf[0..REGISTER_LEN].copy_from_slice(&reg.to_le_bytes());
            Ok((REGISTER_LEN, &mut buf[REGISTER_LEN..]))
        } else {
            Ok((0, buf))
        }
    }

    /// Encodes a u16 value into a slice, prefixed by a length
    /// Returns the number of bytes written and the remaining buffer
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    fn encode_value<T: Into<u16>>(buf: &mut [u8], value: T) -> Result<(usize, &mut [u8]), Error> {
        if buf.len() < LENGTH_VALUE_LEN {
            return Err(Error::InvalidSize(InvalidSizeError {
                expected: LENGTH_VALUE_LEN,
                actual: buf.len(),
            }));
        }
        // Length value includes the size of the length as well
        buf[0..VALUE_LEN].copy_from_slice(&4u16.to_le_bytes());
        buf[VALUE_LEN..LENGTH_VALUE_LEN].copy_from_slice(&value.into().to_le_bytes());
        Ok((LENGTH_VALUE_LEN, &mut buf[LENGTH_VALUE_LEN..]))
    }

    /// Encodes data into a slice, prefixed by a length
    /// Returns the number of bytes written and the remaining buffer
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    fn encode_data<'b>(buf: &'b mut [u8], data: &[u8]) -> Result<(usize, &'b mut [u8]), Error> {
        // +2 to encode the length of the data
        let total_len = data.len() + 2;
        if buf.len() < total_len {
            return Err(Error::InvalidSize(InvalidSizeError {
                expected: total_len,
                actual: buf.len(),
            }));
        }

        buf[0..VALUE_LEN].copy_from_slice(&(total_len as u16).to_le_bytes());
        buf[VALUE_LEN..data.len() + VALUE_LEN].copy_from_slice(data);
        Ok((total_len, &mut buf[data.len()..]))
    }

    /// Encode the command into a slice, returns number of bytes written
    /// If command_reg or data_reg is provided, those addresses will be encoded into the buffer as well
    pub fn encode_into_slice(
        &self,
        buf: &mut [u8],
        command_reg: Option<u16>,
        data_reg: Option<u16>,
    ) -> Result<usize, Error> {
        let mut len = 0;
        let buf: &mut [u8] = buf;

        // Encode command register address
        let (command_len, buf) = Self::encode_register(buf, command_reg)?;
        len += command_len;

        match self {
            Command::Reset => {
                let (command_len, _) = Self::encode_basic_op(buf, Opcode::Reset)?;
                len += command_len;
            }
            Command::GetReport(report_type, report_id) => {
                let (command_len, buf) = Self::encode_common(buf, Opcode::GetReport, Some(*report_type), *report_id)?;
                len += command_len;

                // Encode data register address
                let (register_len, _) = Self::encode_register(buf, data_reg)?;
                len += register_len;
            }
            Command::SetReport(report_type, report_id, data) => {
                let borrow = data.borrow().map_err(|_| Error::InvalidData)?;
                let data: &[u8] = borrow.borrow();

                let (command_len, buf) = Self::encode_common(buf, Opcode::SetReport, Some(*report_type), *report_id)?;
                len += command_len;

                // Encode data register address
                let (register_len, buf) = Self::encode_register(buf, data_reg)?;
                len += register_len;

                // Encode report data
                let (data_len, _) = Self::encode_data(buf, data)?;
                len += data_len
            }
            Command::GetIdle(report_id) => {
                let (command_len, buf) = Self::encode_common(buf, Opcode::GetIdle, None, *report_id)?;
                len += command_len;

                // Encode data register address
                let (register_len, _) = Self::encode_register(buf, data_reg)?;
                len += register_len;
            }
            Command::SetIdle(report_id, freq) => {
                let (command_len, buf) = Self::encode_common(buf, Opcode::SetIdle, None, *report_id)?;
                len += command_len;

                // Encode data register address
                let (register_len, buf) = Self::encode_register(buf, data_reg)?;
                len += register_len;

                // Include data length
                let (data_len, _) = Self::encode_value(buf, *freq)?;
                len += data_len;
            }
            Command::GetProtocol => {
                let (command_len, buf) = Self::encode_basic_op(buf, Opcode::GetProtocol)?;
                len += command_len;

                // Encode data register address
                let (register_len, _) = Self::encode_register(buf, data_reg)?;
                len += register_len;
            }
            Command::SetProtocol(protocol) => {
                let (command_len, buf) = Self::encode_basic_op(buf, Opcode::SetProtocol)?;
                len += command_len;

                // Encode data register address
                let (register_len, buf) = Self::encode_register(buf, data_reg)?;
                len += register_len;

                // Encode data
                let (data_len, _) = Self::encode_value(buf, *protocol)?;
                len += data_len;
            }
            Command::SetPower(state) => {
                let opcode: u16 = Opcode::SetPower.into();
                let state: u16 = (*state).into();
                let buf_len = buf.len();
                buf.get_mut(0..2)
                    .ok_or(Error::InvalidSize(InvalidSizeError {
                        expected: 2,
                        actual: buf_len,
                    }))?
                    .copy_from_slice(&(opcode | state).to_le_bytes());
                len += 2;
            }
            Command::Vendor => {
                let (command_len, _) = Self::encode_basic_op(buf, Opcode::Vendor)?;
                len += command_len;
            }
        }

        Ok(len)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::define_static_buffer;

    const CMD_REG: u16 = 0x0005;
    const DATA_REG: u16 = 0x0006;
    const REPORT_ID: ReportId = ReportId(8);
    const EXT_REPORT_ID: ReportId = ReportId(EXTENDED_REPORT_ID);

    #[test]
    fn test_serialize_reset() {
        let mut test_buffer = [0u8; 4];

        // Test basic functionality
        let len = Command::Reset.encode_into_slice(&mut test_buffer, None, None).unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x01]);

        // Test with command register
        test_buffer.fill(0);
        let len = Command::Reset
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x00, 0x1]);
    }

    #[test]
    fn test_serialize_get_report() {
        let mut test_buffer = [0u8; 7];

        // Test input report
        let len = Command::GetReport(ReportType::Input, REPORT_ID)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x18, 0x02]);

        // Test feature report
        test_buffer.fill(0);
        let len = Command::GetReport(ReportType::Feature, REPORT_ID)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x38, 0x02]);

        // Test extended report
        test_buffer.fill(0);
        let len = Command::GetReport(ReportType::Input, EXT_REPORT_ID)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x1f, 0x02, EXTENDED_REPORT_ID]);

        // Test standard report id with registers
        test_buffer.fill(0);
        let len = Command::GetReport(ReportType::Feature, REPORT_ID)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x38, 0x02, 0x06, 0x00]);

        // Test extended report id with registers
        test_buffer.fill(0);
        let len = Command::GetReport(ReportType::Input, EXT_REPORT_ID)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x1f, 0x02, EXTENDED_REPORT_ID, 0x06, 0x00]
        );
    }

    #[test]
    fn test_serialize_set_report() {
        let mut test_buffer = [0u8; 11];
        define_static_buffer!(data_buffer, u8, [0x00, 0x00]);

        let data = data_buffer::get_mut().unwrap();

        // Test output report
        let len = Command::SetReport(ReportType::Output, REPORT_ID, data.reference())
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x28, 0x03, 0x04, 0x00, 0x00, 0x00]);

        // Test feature report
        test_buffer.fill(0);
        let len = Command::SetReport(ReportType::Feature, REPORT_ID, data.reference())
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x38, 0x03, 0x04, 0x00, 0x00, 0x00]);

        // Test extended report
        test_buffer.fill(0);
        let len = Command::SetReport(ReportType::Output, EXT_REPORT_ID, data.reference())
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x2f, 0x03, EXTENDED_REPORT_ID, 0x04, 0x00, 0x00, 0x00]
        );

        // Test standard report id with registers
        test_buffer.fill(0);
        let len = Command::SetReport(ReportType::Output, REPORT_ID, data.reference())
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x28, 0x03, 0x06, 0x00, 0x04, 0x00, 0x00, 0x00]
        );

        // Test extended report id with registers
        test_buffer.fill(0);
        let len = Command::SetReport(ReportType::Output, EXT_REPORT_ID, data.reference())
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [
                0x05,
                0x00,
                0x2f,
                0x03,
                EXTENDED_REPORT_ID,
                0x06,
                0x00,
                0x04,
                0x00,
                0x00,
                0x00
            ]
        );
    }

    #[test]
    fn test_serialize_get_idle() {
        let mut test_buffer = [0u8; 7];

        // Test standard report id
        let len = Command::GetIdle(REPORT_ID)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x08, 0x04]);

        // Test extended report id
        test_buffer.fill(0);
        let len = Command::GetIdle(EXT_REPORT_ID)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x0f, 0x04, EXTENDED_REPORT_ID]);

        // Test standard report id with registers
        test_buffer.fill(0);
        let len = Command::GetIdle(REPORT_ID)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x08, 0x04, 0x06, 0x00]);

        // Test extended report id with registers
        test_buffer.fill(0);
        let len = Command::GetIdle(EXT_REPORT_ID)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x0f, 0x04, EXTENDED_REPORT_ID, 0x06, 0x00]
        );
    }

    #[test]
    fn test_serialize_set_idle() {
        let mut test_buffer = [0u8; 11];

        // Test standard report id
        let len = Command::SetIdle(REPORT_ID, ReportFreq::Infinite)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x08, 0x05, 0x04, 0x00, 0x00, 0x00]);

        test_buffer.fill(0);
        let len = Command::SetIdle(REPORT_ID, ReportFreq::Msecs(0x0203))
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x08, 0x05, 0x04, 0x00, 0x03, 0x02]);

        // Test extended report id
        test_buffer.fill(0);
        let len = Command::SetIdle(EXT_REPORT_ID, ReportFreq::Infinite)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x0f, 0x05, EXTENDED_REPORT_ID, 0x04, 0x00, 0x00, 0x00]
        );

        test_buffer.fill(0);
        let len = Command::SetIdle(EXT_REPORT_ID, ReportFreq::Msecs(0x0203))
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x0f, 0x05, EXTENDED_REPORT_ID, 0x04, 0x00, 0x03, 0x02]
        );

        // Test standard report id with registers
        test_buffer.fill(0);
        let len = Command::SetIdle(REPORT_ID, ReportFreq::Infinite)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x08, 0x05, 0x06, 0x00, 0x04, 0x00, 0x00, 0x00]
        );

        test_buffer.fill(0);
        let len = Command::SetIdle(REPORT_ID, ReportFreq::Msecs(0x0203))
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x08, 0x05, 0x06, 0x00, 0x04, 0x00, 0x03, 0x02]
        );

        // Test extended report id with registers
        test_buffer.fill(0);
        let len = Command::SetIdle(EXT_REPORT_ID, ReportFreq::Infinite)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [
                0x05,
                0x00,
                0x0f,
                0x05,
                EXTENDED_REPORT_ID,
                0x06,
                0x00,
                0x04,
                0x00,
                0x00,
                0x00
            ]
        );

        test_buffer.fill(0);
        let len = Command::SetIdle(EXT_REPORT_ID, ReportFreq::Msecs(0x0203))
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [
                0x05,
                0x00,
                0x0f,
                0x05,
                EXTENDED_REPORT_ID,
                0x06,
                0x00,
                0x04,
                0x00,
                0x03,
                0x02
            ]
        );
    }

    #[test]
    fn test_serialize_get_protocol() {
        let mut test_buffer = [0u8; 6];

        // Test basic functionality
        let len = Command::GetProtocol
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x06]);

        // Test with command register
        test_buffer.fill(0);
        let len = Command::GetProtocol
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x00, 0x06, 0x06, 0x00]);
    }

    #[test]
    fn test_serialized_set_protocol() {
        let mut test_buffer = [0u8; 10];

        // Test basic functionality
        let len = Command::SetProtocol(Protocol::Boot)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x07, 0x04, 0x00, 0x00, 0x00]);

        test_buffer.fill(0);
        let len = Command::SetProtocol(Protocol::Report)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x07, 0x04, 0x00, 0x01, 0x00]);

        // Test with registers
        test_buffer.fill(0);
        let len = Command::SetProtocol(Protocol::Boot)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x00, 0x07, 0x06, 0x00, 0x04, 0x00, 0x00, 0x00]
        );

        test_buffer.fill(0);
        let len = Command::SetProtocol(Protocol::Report)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), Some(DATA_REG))
            .unwrap();
        assert_eq!(
            &test_buffer[0..len],
            [0x05, 0x00, 0x00, 0x07, 0x06, 0x00, 0x04, 0x00, 0x01, 0x00]
        );
    }

    #[test]
    fn test_serialized_set_power() {
        let mut test_buffer = [0u8; 4];

        // Test basic functionality
        let len = Command::SetPower(PowerState::On)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x08]);

        test_buffer.fill(0);
        let len = Command::SetPower(PowerState::Sleep)
            .encode_into_slice(&mut test_buffer, None, None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x01, 0x08]);

        // Test with command register
        test_buffer.fill(0);
        let len = Command::SetPower(PowerState::On)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x00, 0x08]);

        test_buffer.fill(0);
        let len = Command::SetPower(PowerState::Sleep)
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x01, 0x08]);
    }

    #[test]
    fn test_serialized_vendor() {
        let mut test_buffer = [0xffu8; 4];

        // Test basic functionality
        test_buffer.fill(0);
        let len = Command::Vendor.encode_into_slice(&mut test_buffer, None, None).unwrap();
        assert_eq!(&test_buffer[0..len], [0x00, 0x0e]);

        // Test with command register
        test_buffer.fill(0);
        let len = Command::Vendor
            .encode_into_slice(&mut test_buffer, Some(CMD_REG), None)
            .unwrap();
        assert_eq!(&test_buffer[0..len], [0x05, 0x00, 0x00, 0x0e]);
    }
}
