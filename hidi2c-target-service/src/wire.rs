//! Pure wire-format types for the HID-over-I2C protocol.
//!
//! This is the functional core of the service: no bus, no `async`, no HAL, no allocation.
//! Everything here is a function from bytes to values (or values to bytes), so it can be
//! exercised on the host with no mocks and hammered with arbitrary input by `proptest`.
//!
//! The imperative shell lives in [`crate::service`] and is responsible for all I/O.
//!
//! Section references are to the *HID over I2C Protocol Specification*, version 1.0.

use crate::{HidI2cRegister, ProtocolError};
use embedded_services::relay::hid::{self, GetHidReportType, HidReport, HidReportDescriptor, ReportId};

/// Size of the length field that prefixes every report on the wire (section 6.1.2).
pub(crate) const LENGTH_FIELD_BYTES: u16 = 2;

/// Size of the report ID field that follows the length field when the report descriptor
/// defines explicit report IDs (sections 6.1.2, 7.2.2.2, 7.2.3.1).
pub(crate) const REPORT_ID_FIELD_BYTES: u16 = 1;

/// Sentinel value in the command byte's report-ID nibble meaning "the real report ID is in
/// the following byte" (sections 7.2.2.4, 7.2.3.4).
const EXTENDED_REPORT_ID_SENTINEL: u8 = 0x0F;

/// Mask selecting the report-ID nibble of the command register's low byte (section 7.1.1).
const REPORT_ID_NIBBLE_MASK: u8 = 0x0F;

/// Report type encodings in the command register's low byte (section 7.1.1).
const REPORT_TYPE_INPUT: u8 = 0b01;
const REPORT_TYPE_OUTPUT: u8 = 0b10;
const REPORT_TYPE_FEATURE: u8 = 0b11;

/// HID-I2C command opcodes (section 7.1.1).
///
/// The optional `GetIdle`/`SetIdle`/`GetProtocol`/`SetProtocol` commands are listed in the spec
/// as optional and "not sent by modern hosts", so they are deliberately not implemented.
#[repr(u8)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Opcode {
    Reset = 0x01,
    GetReport = 0x02,
    SetReport = 0x03,
    SetPower = 0x08,
}

/// Wire encoding of the HID power states (section 7.2.8).
#[repr(u8)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum I2cPowerState {
    On = 0x00,
    Sleep = 0x01,
}

impl From<I2cPowerState> for hid::HidDevicePowerState {
    fn from(value: I2cPowerState) -> Self {
        match value {
            I2cPowerState::On => hid::HidDevicePowerState::On,
            I2cPowerState::Sleep => hid::HidDevicePowerState::Sleep,
        }
    }
}

/// Whether the report descriptor defines explicit report IDs.
///
/// This single value decides whether a report ID byte appears on the wire, and therefore how
/// many framing bytes precede every report payload. Keeping it as a type rather than a `bool`
/// means the "2 bytes, or 3 when there are report IDs" arithmetic exists in exactly one place
/// instead of being open-coded at each of the five sites that need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum ReportFraming {
    /// A single top-level collection: no report ID appears on the wire.
    Implicit,
    /// Multiple top-level collections: a mandatory 1-byte report ID follows the length field.
    Explicit,
}

impl ReportFraming {
    /// The framing implied by a report descriptor.
    pub(crate) fn of(descriptor: &HidReportDescriptor<'_>) -> Self {
        if descriptor.report_ids_implicit() {
            Self::Implicit
        } else {
            Self::Explicit
        }
    }

    /// Bytes occupied by the report ID field on the wire.
    pub(crate) const fn report_id_bytes(self) -> u16 {
        match self {
            Self::Implicit => 0,
            Self::Explicit => REPORT_ID_FIELD_BYTES,
        }
    }

    /// Total framing bytes preceding the payload.
    ///
    /// This is also the amount by which the wire length field exceeds the payload length,
    /// because that field counts itself and the report ID as well as the payload
    /// (sections 6.1.2, 7.2.2.2, 7.2.3.1).
    pub(crate) const fn header_bytes(self) -> u16 {
        LENGTH_FIELD_BYTES + self.report_id_bytes()
    }
}

/// Report types a host may request with `GET_REPORT`.
///
/// Section 7.2.2.1 restricts the request to `Input (01)` or `Feature (11)`, and section 7.2.2.4
/// says the device shall ignore `Output`. Modelling that as its own type means an `Output`
/// get-request cannot be constructed, so no downstream code has to re-check for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum GetReportType {
    Input,
    Feature,
}

impl GetReportType {
    fn from_command_byte(command_byte: u8) -> Result<Self, ProtocolError> {
        match command_byte >> 4 {
            REPORT_TYPE_INPUT => Ok(Self::Input),
            REPORT_TYPE_FEATURE => Ok(Self::Feature),
            _ => Err(ProtocolError::InvalidReportType),
        }
    }
}

impl From<GetReportType> for GetHidReportType {
    fn from(value: GetReportType) -> Self {
        match value {
            GetReportType::Input => GetHidReportType::Input,
            GetReportType::Feature => GetHidReportType::Feature,
        }
    }
}

/// Report types a host may set with `SET_REPORT`.
///
/// Section 7.2.3.1 restricts the request to `Output (10)` or `Feature (11)`; section 7.2.3.4
/// notes the device may ignore `Input` as meaningless. As with [`GetReportType`], the illegal
/// case is made unrepresentable rather than guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum SetReportType {
    Output,
    Feature,
}

impl SetReportType {
    fn from_command_byte(command_byte: u8) -> Result<Self, ProtocolError> {
        match command_byte >> 4 {
            REPORT_TYPE_OUTPUT => Ok(Self::Output),
            REPORT_TYPE_FEATURE => Ok(Self::Feature),
            _ => Err(ProtocolError::InvalidReportType),
        }
    }
}

/// A fully parsed command from the Command register (section 7.2).
///
/// Producing one of these is the only fallible step in handling a host command; everything
/// downstream operates on values that cannot be malformed.
pub(crate) enum Command<'buf> {
    /// Host-requested device reset (section 7.2.1).
    Reset,
    /// Host-commanded power state change (section 7.2.8).
    SetPower(I2cPowerState),
    /// Host request for a specific report (section 7.2.2).
    GetReport {
        report_type: GetReportType,
        report_id: ReportId,
    },
    /// Host-supplied report to apply to the device (section 7.2.3).
    SetReport {
        report_type: SetReportType,
        report: HidReport<'buf>,
    },
}

impl<'buf> Command<'buf> {
    /// Parse the bytes that follow the Command register address.
    ///
    /// Total in the sense that matters: every possible input either yields a `Command` or a
    /// [`ProtocolError`]. There is no input for which this panics.
    pub(crate) fn parse(frame: &'buf [u8], framing: ReportFraming) -> Result<Self, ProtocolError> {
        let (&command_byte, rest) = frame.split_first().ok_or(ProtocolError::InvalidCommand)?;
        let (&opcode_byte, rest) = rest.split_first().ok_or(ProtocolError::InvalidCommand)?;

        match Opcode::try_from(opcode_byte).map_err(|_| ProtocolError::InvalidCommand)? {
            Opcode::Reset => Ok(Command::Reset),

            Opcode::SetPower => {
                let state = I2cPowerState::try_from(command_byte).map_err(|_| ProtocolError::InvalidCommand)?;
                Ok(Command::SetPower(state))
            }

            Opcode::GetReport => {
                let report_type = GetReportType::from_command_byte(command_byte)?;
                let (report_id, _data) = parse_report_id_and_data_register(command_byte, rest)?;
                Ok(Command::GetReport { report_type, report_id })
            }

            Opcode::SetReport => {
                let report_type = SetReportType::from_command_byte(command_byte)?;
                let (report_id, data) = parse_report_id_and_data_register(command_byte, rest)?;
                let report = parse_report(data, framing)?;

                // The report ID appears twice for explicit framing: once in the command header
                // and once in the data payload. They must agree.
                if framing == ReportFraming::Explicit && report.id() != report_id {
                    return Err(ProtocolError::InvalidData);
                }

                Ok(Command::SetReport {
                    report_type,
                    // Implicit framing is presented to `HidDevice` implementations as report ID 0,
                    // which is what `parse_report` already produced.
                    report,
                })
            }
        }
    }
}

/// Consume the optional extended report-ID byte and the mandatory Data register address that
/// follow the command value (sections 7.2.2.1, 7.2.3.1).
fn parse_report_id_and_data_register(command_byte: u8, rest: &[u8]) -> Result<(ReportId, &[u8]), ProtocolError> {
    let (report_id, rest) = if command_byte & REPORT_ID_NIBBLE_MASK == EXTENDED_REPORT_ID_SENTINEL {
        let (&extended_id, rest) = rest.split_first().ok_or(ProtocolError::InvalidSize)?;
        (ReportId(extended_id), rest)
    } else {
        (ReportId(command_byte & REPORT_ID_NIBBLE_MASK), rest)
    };

    let (&register, rest) = rest
        .split_first_chunk::<{ core::mem::size_of::<u16>() }>()
        .ok_or(ProtocolError::InvalidSize)?;

    if u16::from_le_bytes(register) != HidI2cRegister::Data as u16 {
        return Err(ProtocolError::InvalidRegisterAddress);
    }

    Ok((report_id, rest))
}

/// Parse a report in its on-the-wire form: `[wLength(2)][report ID?][payload]`.
///
/// `wLength` counts itself, the report ID (when the descriptor defines report IDs) and the
/// payload, so the payload length is `wLength - framing.header_bytes()` (sections 6.1.2,
/// 6.2.2, 7.2.3.1).
///
/// Shared by the Output register path (section 6.2.2) and the `SET_REPORT` command path
/// (section 7.2.3), which use identical framing.
pub(crate) fn parse_report(data: &[u8], framing: ReportFraming) -> Result<HidReport<'_>, ProtocolError> {
    let (&length_bytes, data) = data
        .split_first_chunk::<{ core::mem::size_of::<u16>() }>()
        .ok_or(ProtocolError::InvalidSize)?;

    let payload_len = usize::from(
        u16::from_le_bytes(length_bytes)
            .checked_sub(framing.header_bytes())
            .ok_or(ProtocolError::InvalidSize)?,
    );

    let (report_id, data) = match framing {
        // Implicit framing is presented to `HidDevice` implementations as report ID 0.
        ReportFraming::Implicit => (ReportId(0), data),
        ReportFraming::Explicit => {
            let (&report_id, rest) = data.split_first().ok_or(ProtocolError::InvalidSize)?;
            (ReportId(report_id), rest)
        }
    };

    let payload = data.get(..payload_len).ok_or(ProtocolError::InvalidSize)?;
    Ok(HidReport::new(report_id, payload))
}

/// The framing bytes that precede a report payload sent to the host: the length field, plus
/// the report ID when the descriptor defines report IDs (sections 6.1.2, 7.2.2.2).
///
/// Constructing one is the only place the outgoing length arithmetic happens, and it is
/// checked, so an oversized report is reported as [`ProtocolError::InvalidSize`] rather than
/// silently truncated by an `as u16` cast or panicking on overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReportHeader {
    bytes: [u8; 3],
    len: usize,
}

impl ReportHeader {
    /// Build the header for a payload of `payload_len` bytes belonging to report `id`.
    ///
    /// Section 5.1 caps a report at `2^16 - 4` bytes, so any payload whose framed length
    /// would not fit in the `u16` length field is rejected.
    pub(crate) fn new(payload_len: usize, id: ReportId, framing: ReportFraming) -> Result<Self, ProtocolError> {
        let payload_len = u16::try_from(payload_len).map_err(|_| ProtocolError::InvalidSize)?;
        let total_len = payload_len
            .checked_add(framing.header_bytes())
            .ok_or(ProtocolError::InvalidSize)?;

        let [low, high] = total_len.to_le_bytes();
        Ok(match framing {
            ReportFraming::Implicit => Self {
                bytes: [low, high, 0],
                len: LENGTH_FIELD_BYTES as usize,
            },
            ReportFraming::Explicit => Self {
                bytes: [low, high, id.0],
                len: (LENGTH_FIELD_BYTES + REPORT_ID_FIELD_BYTES) as usize,
            },
        })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        // `len` is only ever set to 2 or 3 by `new`, both in bounds of a 3-byte array.
        self.bytes.get(..self.len).unwrap_or(&self.bytes)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const BOTH_FRAMINGS: [ReportFraming; 2] = [ReportFraming::Implicit, ReportFraming::Explicit];

    /// Low byte of the Data register address; every report command must point here.
    const DATA_REG_LOW: u8 = HidI2cRegister::Data as u8;

    // -----------------------------------------------------------------------------------
    // Wire-value to service-value conversions
    // -----------------------------------------------------------------------------------

    /// Section 7.2.8 defines ON as 0x00 and SLEEP as 0x01; both must reach the device unchanged.
    #[test]
    fn power_states_map_onto_the_device_power_states() {
        assert!(matches!(
            hid::HidDevicePowerState::from(I2cPowerState::On),
            hid::HidDevicePowerState::On
        ));
        assert!(matches!(
            hid::HidDevicePowerState::from(I2cPowerState::Sleep),
            hid::HidDevicePowerState::Sleep
        ));
    }

    /// Both GET_REPORT types must survive the hand-off to the device unchanged; getting this
    /// wrong would silently serve a feature report where an input report was asked for.
    #[test]
    fn get_report_types_map_onto_the_device_report_types() {
        assert!(matches!(
            GetHidReportType::from(GetReportType::Input),
            GetHidReportType::Input
        ));
        assert!(matches!(
            GetHidReportType::from(GetReportType::Feature),
            GetHidReportType::Feature
        ));
    }

    // -----------------------------------------------------------------------------------
    // Framing arithmetic
    // -----------------------------------------------------------------------------------

    /// Section 6.1.2: `[length(2)][report ID(1)][report]` when report IDs are defined,
    /// `[length(2)][report]` when they are not.
    #[test]
    fn framing_header_sizes_match_the_spec() {
        assert_eq!(ReportFraming::Implicit.header_bytes(), 2);
        assert_eq!(ReportFraming::Explicit.header_bytes(), 3);
        assert_eq!(ReportFraming::Implicit.report_id_bytes(), 0);
        assert_eq!(ReportFraming::Explicit.report_id_bytes(), 1);
    }

    // -----------------------------------------------------------------------------------
    // Report type nibbles. 256 command bytes is small enough to walk exhaustively, so these
    // are proof rather than sampling.
    // -----------------------------------------------------------------------------------

    /// Section 7.2.2.1 restricts GET_REPORT to Input (01) or Feature (11). Every other
    /// encoding of the type field - including Output - must be rejected.
    #[test]
    fn get_report_accepts_only_input_and_feature_for_every_command_byte() {
        for command_byte in 0u8..=0xff {
            let expected = match command_byte >> 4 {
                0b01 => Some(GetReportType::Input),
                0b11 => Some(GetReportType::Feature),
                _ => None,
            };

            assert_eq!(
                GetReportType::from_command_byte(command_byte).ok(),
                expected,
                "command byte {command_byte:#04x}"
            );
        }
    }

    /// Section 7.2.3.1 restricts SET_REPORT to Output (10) or Feature (11). Input is not
    /// representable, so there is no "host sent us an input report" branch to test.
    #[test]
    fn set_report_accepts_only_output_and_feature_for_every_command_byte() {
        for command_byte in 0u8..=0xff {
            let expected = match command_byte >> 4 {
                0b10 => Some(SetReportType::Output),
                0b11 => Some(SetReportType::Feature),
                _ => None,
            };

            assert_eq!(
                SetReportType::from_command_byte(command_byte).ok(),
                expected,
                "command byte {command_byte:#04x}"
            );
        }
    }

    /// Only the four opcodes we implement may parse; everything else is `InvalidCommand`.
    #[test]
    fn parse_rejects_every_unimplemented_opcode() {
        for opcode in 0u8..=0xff {
            let frame = [0x00, opcode];
            let parsed = Command::parse(&frame, ReportFraming::Implicit);

            match opcode {
                0x01 => assert!(matches!(parsed, Ok(Command::Reset))),
                0x08 => assert!(matches!(parsed, Ok(Command::SetPower(I2cPowerState::On)))),
                // GET/SET_REPORT parse further and run out of bytes rather than rejecting the opcode.
                0x02 | 0x03 => assert!(matches!(parsed, Err(ProtocolError::InvalidReportType))),
                _ => assert!(
                    matches!(parsed, Err(ProtocolError::InvalidCommand)),
                    "opcode {opcode:#04x}"
                ),
            }
        }
    }

    /// Section 7.2.8 defines only ON (0x00) and SLEEP (0x01).
    #[test]
    fn set_power_accepts_only_the_two_defined_states() {
        for command_byte in 0u8..=0xff {
            let frame = [command_byte, Opcode::SetPower as u8];
            let expected = match command_byte {
                0x00 => Some(I2cPowerState::On),
                0x01 => Some(I2cPowerState::Sleep),
                _ => None,
            };

            let actual = match Command::parse(&frame, ReportFraming::Implicit) {
                Ok(Command::SetPower(state)) => Some(state),
                _ => None,
            };

            assert_eq!(actual, expected, "command byte {command_byte:#04x}");
        }
    }

    // -----------------------------------------------------------------------------------
    // Encode/decode agreement
    // -----------------------------------------------------------------------------------

    /// Whatever framing we emit to the host, we must be able to read back. This pins the
    /// length arithmetic on both sides to the same rule.
    #[test]
    fn report_header_round_trips_through_parse_report() {
        for framing in BOTH_FRAMINGS {
            for payload_len in 0..=16usize {
                let payload: Vec<u8> = (0..payload_len).map(|i| 0xa0u8.wrapping_add(i as u8)).collect();
                let id = ReportId(0x21);

                let header = ReportHeader::new(payload.len(), id, framing).unwrap();
                let mut wire = Vec::from(header.as_bytes());
                wire.extend_from_slice(&payload);

                let parsed = parse_report(&wire, framing).unwrap();
                let context = format!("{framing:?}, payload_len {payload_len}");

                assert_eq!(parsed.data(), payload.as_slice(), "{context}");
                assert_eq!(
                    parsed.id(),
                    match framing {
                        ReportFraming::Implicit => ReportId(0),
                        ReportFraming::Explicit => id,
                    },
                    "{context}"
                );
            }
        }
    }

    /// Section 7.2.2.2's worked example: a 4-byte mouse report is advertised as length 0x0006
    /// for a single-collection (implicit) device.
    #[test]
    fn report_header_matches_the_spec_worked_example() {
        let header = ReportHeader::new(4, ReportId(0), ReportFraming::Implicit).unwrap();
        assert_eq!(header.as_bytes(), &[0x06, 0x00]);
    }

    /// Section 5.1 caps a report at `2^16 - 4` bytes. A payload whose framed length would not
    /// fit the 16-bit field is rejected rather than truncated by an `as` cast or overflowing.
    #[test]
    fn report_header_rejects_payloads_too_large_to_frame() {
        for framing in BOTH_FRAMINGS {
            let largest_ok = usize::from(u16::MAX - framing.header_bytes());

            assert!(ReportHeader::new(largest_ok, ReportId(1), framing).is_ok());
            assert_eq!(
                ReportHeader::new(largest_ok + 1, ReportId(1), framing),
                Err(ProtocolError::InvalidSize)
            );
            assert_eq!(
                ReportHeader::new(usize::MAX, ReportId(1), framing),
                Err(ProtocolError::InvalidSize)
            );
        }
    }

    // -----------------------------------------------------------------------------------
    // Length field handling
    // -----------------------------------------------------------------------------------

    /// The wire length counts itself and the report ID, so any value below the framing size is
    /// malformed. Section 6.1.2 says as much: "Length field should always be greater than 3
    /// Bytes for a valid report."
    #[test]
    fn parse_report_rejects_every_length_that_undercounts_its_own_framing() {
        for framing in BOTH_FRAMINGS {
            for wire_len in 0..framing.header_bytes() {
                let [low, high] = wire_len.to_le_bytes();
                let frame = [low, high, 0x01, 0x02, 0x03, 0x04];

                assert_eq!(
                    parse_report(&frame, framing).err(),
                    Some(ProtocolError::InvalidSize),
                    "{framing:?}, wire_len {wire_len}"
                );
            }
        }
    }

    /// A length field that promises more payload than the host actually wrote is rejected
    /// rather than read past the end of the buffer.
    #[test]
    fn parse_report_rejects_length_longer_than_the_supplied_buffer() {
        for framing in BOTH_FRAMINGS {
            // Claim one more payload byte than is present.
            let payload = [0xaa, 0xbb];
            let wire_len = framing.header_bytes() + payload.len() as u16 + 1;
            let [low, high] = wire_len.to_le_bytes();

            let mut frame = vec![low, high];
            if framing == ReportFraming::Explicit {
                frame.push(0x03);
            }
            frame.extend_from_slice(&payload);

            assert_eq!(
                parse_report(&frame, framing).err(),
                Some(ProtocolError::InvalidSize),
                "{framing:?}"
            );
        }
    }

    /// Every 16-bit length value must be handled without panicking, for both framings.
    #[test]
    fn parse_report_handles_every_length_value() {
        let buffer_payload = [0u8; 8];

        for framing in BOTH_FRAMINGS {
            for wire_len in 0u16..=u16::MAX {
                let [low, high] = wire_len.to_le_bytes();
                let mut frame = vec![low, high];
                frame.extend_from_slice(&buffer_payload);

                // Only assertion that matters: this returns rather than panicking.
                let _ = parse_report(&frame, framing);
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Totality
    // -----------------------------------------------------------------------------------

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// `Command::parse` must be total: every byte string either parses or produces a
    /// `ProtocolError`. There is no input for which it panics, reads out of bounds, or
    /// overflows.
    ///
    /// The input space is too large to walk, so this is a fixed-seed deterministic sweep
    /// rather than an exhaustive one - reproducible on failure, and requiring no
    /// property-testing dependency.
    #[test]
    fn command_parse_is_total_over_arbitrary_frames() {
        let mut state = 0x0123_4567_89ab_cdefu64;

        for _ in 0..200_000 {
            let len = (xorshift(&mut state) % 16) as usize;
            let mut frame = Vec::with_capacity(len);
            for _ in 0..len {
                frame.push((xorshift(&mut state) & 0xff) as u8);
            }

            for framing in BOTH_FRAMINGS {
                let _ = Command::parse(&frame, framing);
            }
        }
    }

    /// The same totality requirement, biased towards frames that are structurally plausible -
    /// a valid opcode and Data register address - so the sweep spends its time past the early
    /// rejections rather than bouncing off them.
    #[test]
    fn command_parse_is_total_over_well_formed_looking_frames() {
        let mut state = 0xfedc_ba98_7654_3210u64;
        const OPCODES: [u8; 4] = [
            Opcode::Reset as u8,
            Opcode::GetReport as u8,
            Opcode::SetReport as u8,
            Opcode::SetPower as u8,
        ];

        for _ in 0..200_000 {
            let command_byte = (xorshift(&mut state) & 0xff) as u8;
            let opcode = OPCODES
                .get((xorshift(&mut state) % OPCODES.len() as u64) as usize)
                .copied()
                .unwrap_or(Opcode::Reset as u8);

            let mut frame = vec![command_byte, opcode];
            if command_byte & REPORT_ID_NIBBLE_MASK == EXTENDED_REPORT_ID_SENTINEL {
                frame.push((xorshift(&mut state) & 0xff) as u8);
            }
            frame.extend_from_slice(&[DATA_REG_LOW, 0x00]);

            let tail = (xorshift(&mut state) % 10) as usize;
            for _ in 0..tail {
                frame.push((xorshift(&mut state) & 0xff) as u8);
            }

            for framing in BOTH_FRAMINGS {
                let _ = Command::parse(&frame, framing);
            }
        }
    }

    /// Section 7.2.2.4: a report-ID nibble of `0b1111` is a sentinel promising the real report
    /// ID in a following byte. A frame that ends instead is malformed, not a panic.
    #[test]
    fn extended_report_id_sentinel_without_its_byte_is_rejected() {
        for opcode in [Opcode::GetReport, Opcode::SetReport] {
            // 0x3f: Feature report type, report-ID nibble 0xF - but nothing follows.
            let frame = [0x3f, opcode as u8];

            for framing in BOTH_FRAMINGS {
                assert_eq!(
                    Command::parse(&frame, framing).err(),
                    Some(ProtocolError::InvalidSize),
                    "{opcode:?}, {framing:?}"
                );
            }
        }
    }

    /// The Data register address is mandatory in both report commands (sections 7.2.2.1,
    /// 7.2.3.1); pointing anywhere else is a protocol violation.
    #[test]
    fn report_commands_require_the_data_register_address() {
        for register in 0u16..=0x00ff {
            let [low, high] = register.to_le_bytes();
            let frame = [0x23, Opcode::SetReport as u8, low, high, 0x04, 0x00, 0x03, 0x5a];

            let parsed = Command::parse(&frame, ReportFraming::Explicit);

            if register == HidI2cRegister::Data as u16 {
                assert!(
                    matches!(parsed, Ok(Command::SetReport { .. })),
                    "register {register:#06x}"
                );
            } else {
                assert_eq!(
                    parsed.err(),
                    Some(ProtocolError::InvalidRegisterAddress),
                    "register {register:#06x}"
                );
            }
        }
    }
}
