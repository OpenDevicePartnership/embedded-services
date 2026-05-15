//! DSP0253 byte-stuffed serial medium for MCTP.
//!
//! Two-layer split:
//!   - [`SerialEncoding`]: stateless byte-stuffing (0x7E, 0x7D escape pair).
//!   - [`MctpSerialMedium`]: framing (revision byte, byte_count, body, FCS-16, end-flag).
//!
//! Both layers are gated behind the `serial` cargo feature.

use crate::{
    MctpPacketError,
    buffer_encoding::{BufferEncoding, DecodeError, EncodeError, EncodingDecoder, EncodingEncoder},
    error::MctpPacketResult,
    medium::{MctpMedium, MctpMediumFrame},
};

/// DSP0253 byte-stuffing transform. Stateless ZST.
///
/// Encode: `0x7E -> [0x7D, 0x5E]`, `0x7D -> [0x7D, 0x5D]`, any other
/// byte -> `[b]`.
/// Decode: `0x7D 0x5E -> 0x7E`, `0x7D 0x5D -> 0x7D`, `0x7D <other>` ->
/// `InvalidEscape`.
///
/// Raw `0x7E` in the wire stream is NOT rejected here — that's a
/// framing concern owned by `MctpSerialMedium::deserialize`, which
/// checks the body region for stray flags.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SerialEncoding;

impl BufferEncoding for SerialEncoding {
    fn write_byte(wire_buf: &mut [u8], byte: u8) -> Result<usize, EncodeError> {
        match byte {
            0x7E => {
                if wire_buf.len() < 2 {
                    return Err(EncodeError::BufferFull);
                }
                wire_buf[0] = 0x7D;
                wire_buf[1] = 0x5E;
                Ok(2)
            }
            0x7D => {
                if wire_buf.len() < 2 {
                    return Err(EncodeError::BufferFull);
                }
                wire_buf[0] = 0x7D;
                wire_buf[1] = 0x5D;
                Ok(2)
            }
            b => match wire_buf.first_mut() {
                Some(slot) => {
                    *slot = b;
                    Ok(1)
                }
                None => Err(EncodeError::BufferFull),
            },
        }
    }

    fn read_byte(wire_buf: &[u8]) -> Result<(u8, usize), DecodeError> {
        match wire_buf.first().copied() {
            None => Err(DecodeError::PrematureEnd),
            Some(0x7D) => match wire_buf.get(1).copied() {
                None => Err(DecodeError::PrematureEnd),
                Some(0x5E) => Ok((0x7E, 2)),
                Some(0x5D) => Ok((0x7D, 2)),
                Some(_) => Err(DecodeError::InvalidEscape),
            },
            // Raw 0x7E falls through here as a 1-byte read; the framing
            // layer (`MctpSerialMedium::deserialize`) rejects bare
            // 0x7E inside the body region.
            Some(b) => Ok((b, 1)),
        }
    }

    fn wire_size_of(decoded: &[u8]) -> usize {
        decoded
            .iter()
            .map(|&b| if b == 0x7E || b == 0x7D { 2 } else { 1 })
            .sum()
    }
}

/// SP MCTP endpoint id per CONTEXT D-D-06.
pub const SP_EID: crate::endpoint_id::EndpointId = crate::endpoint_id::EndpointId::Id(0x08);
/// EC MCTP endpoint id per CONTEXT D-D-06.
pub const EC_EID: crate::endpoint_id::EndpointId = crate::endpoint_id::EndpointId::Id(0x0A);
/// Maximum DSP0253 packet body size (DECODED bytes, before stuffing).
pub const CONST_MTU: usize = 251;

const SERIAL_REVISION: u8 = 0x01;
const END_FLAG: u8 = 0x7E;
/// Header bytes: revision + byte_count (decoded body byte count).
const HEADER_LEN: usize = 2;
/// Worst-case trailer wire bytes: 2 stuffed FCS bytes (each may
/// expand 1 -> 2) + 1 end-flag.
const MAX_TRAILER_WIRE: usize = 5;

// CRC-16/X-25 per DSP0253 §8 (poly 0x1021, init 0xFFFF, refin/refout,
// xorout 0xFFFF). Algorithm catalog entry locked in CONTEXT D-D-02.
// FCS bytes on the wire are MSB-first per DSP0253 §5.2 (overrides
// RFC1662's LSB-first PPP convention).
const FCS_ALGO: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_IBM_SDLC);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpSerialMediumFrame {
    pub revision: u8,
    /// DECODED body byte count per DSP0253 §6.2 (NOT the wire byte
    /// count). Cap = `CONST_MTU` = 251; max u8 = 255, fits comfortably.
    pub byte_count: u8,
    pub fcs: u16,
}

impl MctpMediumFrame<MctpSerialMedium> for MctpSerialMediumFrame {
    fn packet_size(&self) -> usize {
        // packet_size is the DECODED body byte count — the contract
        // used by `MctpPacketContext::deserialize_packet`, which then
        // subtracts 4 for the transport header.
        self.byte_count as usize
    }

    fn reply_context(&self) {}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpSerialMedium;

impl MctpMedium for MctpSerialMedium {
    type Frame = MctpSerialMediumFrame;
    type Error = &'static str;
    type ReplyContext = ();
    type Encoding = SerialEncoding;

    fn max_message_body_size(&self) -> usize {
        CONST_MTU
    }

    fn deserialize<'buf>(
        &self,
        packet: &'buf [u8],
    ) -> MctpPacketResult<(Self::Frame, EncodingDecoder<'buf, Self::Encoding>), Self> {
        // Minimum frame: 2 header + 0 body + 2 FCS (unstuffed) + 1 end-flag = 5 bytes.
        if packet.len() < HEADER_LEN + 3 {
            return Err(MctpPacketError::MediumError("packet too short for serial frame"));
        }
        let revision = packet[0];
        if revision != SERIAL_REVISION {
            return Err(MctpPacketError::MediumError("unsupported serial revision"));
        }
        let byte_count = packet[1];
        if (byte_count as usize) > CONST_MTU {
            return Err(MctpPacketError::MediumError("byte_count exceeds MTU"));
        }

        // Single forward walk: un-stuff body bytes (count must equal
        // `byte_count`), un-stuff 2 FCS bytes, expect end-flag, compare
        // CRC.
        let body_wire_start = HEADER_LEN;
        let mut decoded = [0u8; CONST_MTU];
        let mut decoded_len = 0usize;
        let mut wire_pos = 0usize; // offset from body_wire_start

        while decoded_len < byte_count as usize {
            let (b, n) = SerialEncoding::read_byte(&packet[body_wire_start + wire_pos..]).map_err(|e| match e {
                DecodeError::PrematureEnd => MctpPacketError::MediumError("premature end in body"),
                DecodeError::InvalidEscape => MctpPacketError::MediumError("invalid escape in body"),
            })?;
            if b == END_FLAG && n == 1 {
                // Bare (unstuffed) 0x7E inside the body region is a
                // protocol error (MEDIUM-05). A decoded 0x7E whose wire
                // representation was the stuffed pair `0x7D 0x5E`
                // (n==2) is a legitimate payload byte and is kept.
                return Err(MctpPacketError::MediumError("unexpected 0x7E in body"));
            }
            decoded[decoded_len] = b;
            decoded_len += 1;
            wire_pos += n;
        }
        let body_wire_end = body_wire_start + wire_pos;

        // Un-stuff 2 FCS bytes (DSP0253 §7.1 stuffing applies to FCS).
        let (fcs_msb, n_msb) = SerialEncoding::read_byte(&packet[body_wire_end..])
            .map_err(|_| MctpPacketError::MediumError("invalid escape in fcs"))?;
        let (fcs_lsb, n_lsb) = SerialEncoding::read_byte(&packet[body_wire_end + n_msb..])
            .map_err(|_| MctpPacketError::MediumError("invalid escape in fcs"))?;
        let trailer_pos = body_wire_end + n_msb + n_lsb;

        if trailer_pos >= packet.len() || packet[trailer_pos] != END_FLAG {
            return Err(MctpPacketError::MediumError("missing end flag"));
        }
        if trailer_pos + 1 != packet.len() {
            return Err(MctpPacketError::MediumError("trailing bytes after end flag"));
        }

        // FCS-16/X-25 over un-stuffed (revision || byte_count || decoded body).
        let mut digest = FCS_ALGO.digest();
        digest.update(&[revision, byte_count]);
        digest.update(&decoded[..decoded_len]);
        let computed_fcs = digest.finalize();
        // DSP0253 §5.2: MSB first on wire.
        let wire_fcs = u16::from_be_bytes([fcs_msb, fcs_lsb]);
        if wire_fcs != computed_fcs {
            return Err(MctpPacketError::MediumError("fcs mismatch"));
        }

        Ok((
            MctpSerialMediumFrame {
                revision,
                byte_count,
                fcs: wire_fcs,
            },
            EncodingDecoder::<Self::Encoding>::new(&packet[body_wire_start..body_wire_end]),
        ))
    }

    fn serialize<'buf, F>(
        &self,
        _reply_context: Self::ReplyContext,
        buffer: &'buf mut [u8],
        message_writer: F,
    ) -> MctpPacketResult<&'buf [u8], Self>
    where
        F: for<'a> FnOnce(&mut EncodingEncoder<'a, Self::Encoding>) -> MctpPacketResult<(), Self>,
    {
        if buffer.len() < HEADER_LEN + MAX_TRAILER_WIRE {
            return Err(MctpPacketError::MediumError("buffer too small for serial frame"));
        }
        let buffer_len = buffer.len();

        // Run closure over body region (reserve worst-case 5-byte
        // trailer). The encoder stuffs body bytes via
        // `SerialEncoding::write_byte` automatically.
        let body_wire_len = {
            let body_buf = &mut buffer[HEADER_LEN..buffer_len - MAX_TRAILER_WIRE];
            let mut encoder = EncodingEncoder::<Self::Encoding>::new(body_buf);
            message_writer(&mut encoder)?;
            encoder.wire_position()
        };

        // Re-decode body to recover DECODED bytes + decoded count for
        // `byte_count` and FCS. CONTEXT D-B-02 acknowledges the
        // double-walk; ~250 bytes max, no_std, cheap.
        let mut decoded = [0u8; CONST_MTU];
        let mut decoded_len = 0usize;
        let mut wire_pos = 0usize;
        while wire_pos < body_wire_len {
            let (b, n) = SerialEncoding::read_byte(&buffer[HEADER_LEN + wire_pos..HEADER_LEN + body_wire_len])
                .map_err(|_| MctpPacketError::MediumError("internal: failed to re-decode body"))?;
            if decoded_len >= CONST_MTU {
                return Err(MctpPacketError::MediumError("body exceeds MTU"));
            }
            decoded[decoded_len] = b;
            decoded_len += 1;
            wire_pos += n;
        }
        // Should not fire — `EncodingEncoder::write` returns
        // `BufferFull` long before decoded_len could exceed 251.
        if decoded_len > u8::MAX as usize {
            return Err(MctpPacketError::MediumError("body exceeds byte_count u8 cap"));
        }
        let byte_count = decoded_len as u8;

        // FCS-16/X-25 over un-stuffed (revision || byte_count || decoded body).
        let mut digest = FCS_ALGO.digest();
        digest.update(&[SERIAL_REVISION, byte_count]);
        digest.update(&decoded[..decoded_len]);
        let fcs = digest.finalize();
        // DSP0253 §5.2: MSB first on wire.
        let [fcs_msb, fcs_lsb] = fcs.to_be_bytes();

        // Header: revision + byte_count emitted directly (NOT stuffed),
        // matching `SmbusEspiMedium`'s header pattern. See PLAN
        // <behavior> note for the conformance caveat when byte_count
        // happens to equal 0x7E or 0x7D — round-trips cleanly through
        // this implementation's deserialize.
        buffer[0] = SERIAL_REVISION;
        buffer[1] = byte_count;

        // Stuff and write FCS bytes via SerialEncoding (DSP0253 §7.1 +
        // CONTEXT D-B-02 — deserialize un-stuffs FCS, so serialize
        // must stuff).
        let fcs_start = HEADER_LEN + body_wire_len;
        let n_msb = SerialEncoding::write_byte(&mut buffer[fcs_start..], fcs_msb)
            .map_err(|_| MctpPacketError::MediumError("internal: failed to encode fcs"))?;
        let n_lsb = SerialEncoding::write_byte(&mut buffer[fcs_start + n_msb..], fcs_lsb)
            .map_err(|_| MctpPacketError::MediumError("internal: failed to encode fcs"))?;
        let end_pos = fcs_start + n_msb + n_lsb;

        // End-flag is written directly (flags are NOT stuffed by
        // definition).
        buffer[end_pos] = END_FLAG;

        Ok(&buffer[..end_pos + 1])
    }

    fn frame_complete(&self, buf: &[u8]) -> MctpPacketResult<Option<usize>, Self> {
        // This medium's `serialize` emits `[revision, byte_count, ...stuffed body,
        // fcs_msb, fcs_lsb, 0x7E]` — only an END flag, no leading START flag.
        //
        // The `body_start` check below is forward-defensive: DSP0253 variants are
        // permitted to emit a leading 0x7E flag, and this lets `frame_complete`
        // accept that case too (skip the leading flag, then find the next 0x7E).
        // Either pattern returns the full frame length to the caller.
        if buf.is_empty() {
            return Ok(None);
        }
        let body_start = if buf[0] == END_FLAG { 1 } else { 0 };
        if body_start >= buf.len() {
            return Ok(None);
        }
        match buf[body_start..].iter().position(|&b| b == END_FLAG) {
            Some(idx) => Ok(Some(body_start + idx + 1)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    use crate::buffer_encoding::EncodingDecoder;

    #[test]
    fn write_byte_stuffs_7e() {
        let mut buf = [0u8; 4];
        let n = SerialEncoding::write_byte(&mut buf, 0x7E).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[0x7D, 0x5E]);
    }

    #[test]
    fn write_byte_stuffs_7d() {
        let mut buf = [0u8; 4];
        let n = SerialEncoding::write_byte(&mut buf, 0x7D).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[0x7D, 0x5D]);
    }

    #[test]
    fn write_byte_passthrough_plain() {
        let mut buf = [0u8; 1];
        let n = SerialEncoding::write_byte(&mut buf, 0x41).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf, [0x41]);
    }

    #[test]
    fn write_byte_full_buffer_plain() {
        let mut buf = [];
        assert_eq!(
            SerialEncoding::write_byte(&mut buf, 0x41).unwrap_err(),
            EncodeError::BufferFull
        );
    }

    #[test]
    fn write_byte_full_buffer_escape() {
        let mut buf = [0u8; 1];
        assert_eq!(
            SerialEncoding::write_byte(&mut buf, 0x7E).unwrap_err(),
            EncodeError::BufferFull
        );
    }

    #[test]
    fn read_byte_unstuffs_7e() {
        assert_eq!(SerialEncoding::read_byte(&[0x7D, 0x5E]).unwrap(), (0x7E, 2));
    }

    #[test]
    fn read_byte_unstuffs_7d() {
        assert_eq!(SerialEncoding::read_byte(&[0x7D, 0x5D]).unwrap(), (0x7D, 2));
    }

    #[test]
    fn read_byte_passthrough_plain() {
        assert_eq!(SerialEncoding::read_byte(&[0x41]).unwrap(), (0x41, 1));
    }

    #[test]
    fn read_byte_raw_7e_passes_through() {
        // Raw 0x7E is NOT rejected at the encoding layer — framing is
        // the framing layer's concern.
        assert_eq!(SerialEncoding::read_byte(&[0x7E]).unwrap(), (0x7E, 1));
    }

    #[test]
    fn read_byte_premature_end_empty() {
        assert_eq!(SerialEncoding::read_byte(&[]).unwrap_err(), DecodeError::PrematureEnd);
    }

    #[test]
    fn read_byte_premature_end_after_escape() {
        assert_eq!(
            SerialEncoding::read_byte(&[0x7D]).unwrap_err(),
            DecodeError::PrematureEnd
        );
    }

    #[test]
    fn read_byte_invalid_escape() {
        assert_eq!(
            SerialEncoding::read_byte(&[0x7D, 0xAA]).unwrap_err(),
            DecodeError::InvalidEscape
        );
    }

    #[test]
    fn wire_size_of_mixed() {
        assert_eq!(SerialEncoding::wire_size_of(&[0x41, 0x7E, 0x42, 0x7D, 0x43]), 7);
    }

    #[test]
    fn wire_size_of_empty() {
        assert_eq!(SerialEncoding::wire_size_of(&[]), 0);
    }

    #[test]
    fn roundtrip_all_byte_values() {
        // 256-byte payload of every byte value, encoded into a 512-byte
        // wire buffer (worst case is 2x expansion if every byte stuffs;
        // actual expansion here is 256 + 2 = 258 wire bytes).
        let mut decoded = [0u8; 256];
        for (i, slot) in decoded.iter_mut().enumerate() {
            *slot = i as u8;
        }
        let mut wire = [0u8; 512];
        let mut wpos = 0usize;
        for &b in &decoded {
            wpos += SerialEncoding::write_byte(&mut wire[wpos..], b).unwrap();
        }
        assert_eq!(wpos, SerialEncoding::wire_size_of(&decoded));
        let mut dec = EncodingDecoder::<SerialEncoding>::new(&wire[..wpos]);
        for &expected in &decoded {
            assert_eq!(dec.read().unwrap(), expected);
        }
        assert_eq!(dec.read().unwrap_err(), DecodeError::PrematureEnd);
    }
}

#[cfg(test)]
mod fixtures {
    //! Hand-authored DSP0253 serial frame fixtures (golden vectors).
    //!
    //! Layout per fixture (no leading flag — this implementation omits
    //! the open `0x7E` per CONTEXT D-D-01; upstream UART layer supplies
    //! it in Phase 27):
    //!
    //!   `[REVISION=0x01, byte_count, ...stuffed body..., ...stuffed FCS-MSB..., ...stuffed
    //! FCS-LSB..., 0x7E]`
    //!
    //! - Header bytes (REVISION, byte_count) are NOT stuffed (matches production serialize).
    //! - Body bytes are stuffed per `SerialEncoding`.
    //! - FCS-16/X-25 computed over un-stuffed `[REVISION, byte_count, ...decoded body...]`, emitted
    //!   MSB-first on wire (DSP0253 §5.2), each FCS byte then stuffed if equal to 0x7E or 0x7D.
    //! - Trailing `0x7E` is the end-flag (not stuffed by definition).

    pub(crate) const FIXTURE_BASIC_RX: &[u8] = &[0x01, 0x04, 0xAA, 0xBB, 0xCC, 0xDD, 0x6D, 0xA1, 0x7E];

    pub(crate) const FIXTURE_PAYLOAD_CONTAINS_7E: &[u8] = &[0x01, 0x03, 0xAA, 0x7D, 0x5E, 0xCC, 0xFB, 0xE7, 0x7E];

    pub(crate) const FIXTURE_PAYLOAD_CONTAINS_7D: &[u8] = &[0x01, 0x03, 0xAA, 0x7D, 0x5D, 0xCC, 0xD1, 0x8F, 0x7E];

    pub(crate) const FIXTURE_PAYLOAD_CONTAINS_BOTH: &[u8] =
        &[0x01, 0x03, 0x7D, 0x5E, 0x7D, 0x5D, 0x42, 0x50, 0x97, 0x7E];

    /// 251-byte body `(0..251)` decoded; wire = 258 bytes after stuffing
    /// the lone 0x7D (idx 125) and 0x7E (idx 126) inside the body.
    pub(crate) const FIXTURE_MAX_MTU_FRAME: &[u8] = &[
        0x01, 0xFB, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21,
        0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33,
        0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
        0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
        0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B,
        0x7C, 0x7D, 0x5D, 0x7D, 0x5E, 0x7F, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B,
        0x8C, 0x8D, 0x8E, 0x8F, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D,
        0x9E, 0x9F, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
        0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1,
        0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB, 0xCC, 0xCD, 0xCE, 0xCF, 0xD0, 0xD1, 0xD2, 0xD3,
        0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE, 0xDF, 0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5,
        0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xEB, 0xEC, 0xED, 0xEE, 0xEF, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
        0xF8, 0xF9, 0xFA, 0xF6, 0x07, 0x7E,
    ];

    pub(crate) const FIXTURE_EMPTY_PAYLOAD: &[u8] = &[0x01, 0x00, 0x16, 0x9F, 0x7E];

    pub(crate) const FIXTURE_FCS_VALID: &[u8] = &[0x01, 0x03, 0x10, 0x20, 0x30, 0x76, 0xDB, 0x7E];

    /// Same body as FCS_VALID but FCS-MSB byte XOR 0xFF (0x76 -> 0x89).
    pub(crate) const FIXTURE_FCS_INVALID: &[u8] = &[0x01, 0x03, 0x10, 0x20, 0x30, 0x89, 0xDB, 0x7E];

    /// byte_count=2 claims 2 decoded bytes; first body wire byte is
    /// `0x7D 0xAA` (escape followed by non-`{0x5E,0x5D}`) -> rejected
    /// as "invalid escape in body" before reaching FCS.
    pub(crate) const FIXTURE_INVALID_ESCAPE: &[u8] = &[0x01, 0x02, 0x7D, 0xAA, 0x00, 0x00, 0x7E];

    /// byte_count=3, body wire region is `[0xAA, 0x7E, 0xCC]` — the
    /// raw 0x7E inside the body region is rejected before FCS.
    pub(crate) const FIXTURE_PREMATURE_END_FLAG: &[u8] = &[0x01, 0x03, 0xAA, 0x7E, 0xCC, 0x00, 0x00, 0x7E];
}

#[cfg(test)]
mod medium_tests {
    use super::{fixtures::*, *};

    fn drain_decoder(mut dec: EncodingDecoder<'_, SerialEncoding>) -> ([u8; CONST_MTU], usize) {
        let mut out = [0u8; CONST_MTU];
        let mut n = 0;
        while let Ok(b) = dec.read() {
            out[n] = b;
            n += 1;
        }
        (out, n)
    }

    #[test]
    fn decode_basic_rx_succeeds() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_BASIC_RX).unwrap();
        assert_eq!(frame.revision, 0x01);
        assert_eq!(frame.byte_count, 4);
        assert_eq!(frame.fcs, 0x6DA1);
        let (decoded, n) = drain_decoder(dec);
        assert_eq!(&decoded[..n], &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn decode_payload_contains_7e() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_PAYLOAD_CONTAINS_7E).unwrap();
        assert_eq!(frame.byte_count, 3);
        let (decoded, n) = drain_decoder(dec);
        assert_eq!(&decoded[..n], &[0xAA, 0x7E, 0xCC]);
    }

    #[test]
    fn decode_payload_contains_7d() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_PAYLOAD_CONTAINS_7D).unwrap();
        assert_eq!(frame.byte_count, 3);
        let (decoded, n) = drain_decoder(dec);
        assert_eq!(&decoded[..n], &[0xAA, 0x7D, 0xCC]);
    }

    #[test]
    fn decode_payload_contains_both() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_PAYLOAD_CONTAINS_BOTH).unwrap();
        assert_eq!(frame.byte_count, 3);
        let (decoded, n) = drain_decoder(dec);
        assert_eq!(&decoded[..n], &[0x7E, 0x7D, 0x42]);
    }

    #[test]
    fn decode_max_mtu_frame() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_MAX_MTU_FRAME).unwrap();
        assert_eq!(frame.byte_count as usize, CONST_MTU);
        let (decoded, n) = drain_decoder(dec);
        assert_eq!(n, CONST_MTU);
        for (i, &b) in decoded[..n].iter().enumerate() {
            assert_eq!(b, i as u8, "mismatch at idx {i}");
        }
    }

    #[test]
    fn decode_empty_payload() {
        let (frame, dec) = MctpSerialMedium.deserialize(FIXTURE_EMPTY_PAYLOAD).unwrap();
        assert_eq!(frame.byte_count, 0);
        let (_, n) = drain_decoder(dec);
        assert_eq!(n, 0);
    }

    #[test]
    fn decode_fcs_valid() {
        assert!(MctpSerialMedium.deserialize(FIXTURE_FCS_VALID).is_ok());
    }

    #[test]
    fn decode_fcs_invalid_rejects() {
        match MctpSerialMedium.deserialize(FIXTURE_FCS_INVALID) {
            Err(crate::MctpPacketError::MediumError("fcs mismatch")) => {}
            other => panic!("expected MediumError(\"fcs mismatch\"), got {:?}", other.err()),
        }
    }

    #[test]
    fn decode_invalid_escape_rejects() {
        match MctpSerialMedium.deserialize(FIXTURE_INVALID_ESCAPE) {
            Err(crate::MctpPacketError::MediumError("invalid escape in body")) => {}
            other => panic!(
                "expected MediumError(\"invalid escape in body\"), got {:?}",
                other.err()
            ),
        }
    }

    #[test]
    fn decode_premature_end_flag_rejects() {
        match MctpSerialMedium.deserialize(FIXTURE_PREMATURE_END_FLAG) {
            Err(crate::MctpPacketError::MediumError("unexpected 0x7E in body")) => {}
            other => panic!(
                "expected MediumError(\"unexpected 0x7E in body\"), got {:?}",
                other.err()
            ),
        }
    }

    fn fixture_roundtrip(wire: &[u8]) {
        let m = MctpSerialMedium;
        let (_frame, dec) = m.deserialize(wire).unwrap();
        let (decoded, n) = drain_decoder(dec);
        let mut out = [0u8; 1024];
        let serialized = m
            .serialize((), &mut out, |e| {
                e.write_all(&decoded[..n])
                    .map_err(|_| MctpPacketError::MediumError("write failed"))
            })
            .unwrap();
        assert_eq!(serialized, wire);
    }

    #[test]
    fn fixture_roundtrip_basic_rx() {
        fixture_roundtrip(FIXTURE_BASIC_RX);
    }

    #[test]
    fn fixture_roundtrip_payload_contains_7e() {
        fixture_roundtrip(FIXTURE_PAYLOAD_CONTAINS_7E);
    }

    #[test]
    fn fixture_roundtrip_payload_contains_7d() {
        fixture_roundtrip(FIXTURE_PAYLOAD_CONTAINS_7D);
    }

    #[test]
    fn fixture_roundtrip_payload_contains_both() {
        fixture_roundtrip(FIXTURE_PAYLOAD_CONTAINS_BOTH);
    }

    #[test]
    fn fixture_roundtrip_max_mtu_frame() {
        fixture_roundtrip(FIXTURE_MAX_MTU_FRAME);
    }

    #[test]
    fn fixture_roundtrip_empty_payload() {
        fixture_roundtrip(FIXTURE_EMPTY_PAYLOAD);
    }

    #[test]
    fn fixture_roundtrip_fcs_valid() {
        fixture_roundtrip(FIXTURE_FCS_VALID);
    }

    #[test]
    fn public_api_smoke() {
        let _: crate::MctpSerialMedium = crate::MctpSerialMedium;
        let _: crate::SerialEncoding = crate::SerialEncoding;
        assert_eq!(crate::CONST_MTU, 251);
        assert_eq!(crate::SP_EID, crate::EndpointId::Id(0x08));
        assert_eq!(crate::EC_EID, crate::EndpointId::Id(0x0A));
    }

    #[test]
    fn packetize_with_stuffing_respects_mtu() {
        // 251-byte payload of all 0x7E. Each byte stuffs to 2 wire
        // bytes (0x7D 0x5E), so encoded body footprint per packet is
        // 2x decoded length. The packet body MTU is 251 wire bytes;
        // each MCTP packet also carries a 4-byte transport header
        // which itself is `wire_size_of`-measured. Expect the message
        // to split across multiple packets and no body region to
        // exceed CONST_MTU wire bytes.
        use crate::{
            endpoint_id::EndpointId, mctp_message_tag::MctpMessageTag, mctp_packet_context::MctpReplyContext,
            mctp_sequence_number::MctpSequenceNumber, serialize::SerializePacketState,
        };

        let payload = [0x7E_u8; 251];
        let mut assembly = [0u8; 1024];
        let medium = MctpSerialMedium;
        let reply_context = MctpReplyContext::<MctpSerialMedium> {
            destination_endpoint_id: EndpointId::Id(0x0A),
            source_endpoint_id: EndpointId::Id(0x08),
            packet_sequence_number: MctpSequenceNumber::new(0),
            message_tag: MctpMessageTag::default(),
            medium_context: (),
        };
        let mut state = SerializePacketState {
            medium: &medium,
            reply_context,
            current_packet_num: 0,
            serialized_message_header: false,
            message_buffer: &payload[..],
            assembly_buffer: &mut assembly[..],
        };

        let mut total_decoded_body = 0usize;
        let mut packet_count = 0usize;
        loop {
            // We cannot iterate `state.next()` more than once because
            // `next` mutably borrows the assembly buffer for each
            // returned slice. Take one packet, process it, then break.
            let pkt = match state.next() {
                Some(Ok(pkt)) => {
                    let mut tmp = [0u8; 1024];
                    tmp[..pkt.len()].copy_from_slice(pkt);
                    (tmp, pkt.len())
                }
                Some(Err(e)) => panic!("serialize error: {e:?}"),
                None => break,
            };
            packet_count += 1;
            // Deserialize the packet to recover the wire body length
            // and the decoded body byte count.
            let (frame, dec) = medium.deserialize(&pkt.0[..pkt.1]).unwrap();
            // Decoded body byte count INCLUDES the 4 transport-header
            // bytes — subtract to get the actual payload bytes.
            assert!(frame.byte_count as usize >= 4);
            let payload_decoded = frame.byte_count as usize - 4;
            total_decoded_body += payload_decoded;
            // Wire body region (between header and FCS) MUST be <=
            // CONST_MTU under MEDIUM-08 chunk-sizing.
            let _ = dec; // decoder discard
            let wire_body_len = pkt.1 - 2 /* hdr */ - 1 /* end-flag */;
            // Subtract the (possibly stuffed) FCS bytes — they are 2
            // FCS bytes but each may stuff to 2 wire bytes. Worst case
            // 4 bytes; lower bound on body wire = wire_body_len - 4.
            assert!(
                wire_body_len <= CONST_MTU + 4,
                "packet {packet_count} body exceeds MTU + worst-case FCS: {wire_body_len}"
            );
        }
        assert!(packet_count >= 2, "expected multi-packet split, got {packet_count}");
        assert_eq!(total_decoded_body, payload.len());
    }

    // ----- frame_complete tests -----

    #[test]
    fn frame_complete_empty_buf_returns_none() {
        assert_eq!(MctpSerialMedium.frame_complete(&[]).unwrap(), None);
    }

    #[test]
    fn frame_complete_no_end_flag_returns_none() {
        // Bytes present but no 0x7E anywhere → partial frame
        let buf = [0x01, 0x05, 0xAA, 0xBB, 0xCC];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), None);
    }

    #[test]
    fn frame_complete_only_end_flag_returns_none() {
        // A lone 0x7E is treated as a leading flag (skipped); after
        // skipping there are no bytes left, so partial.
        let buf = [END_FLAG];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), None);
    }

    #[test]
    fn frame_complete_no_leading_flag_finds_end_flag() {
        // [ revision, byte_count, ...body, fcs_msb, fcs_lsb, 0x7E ]
        // matches what MctpSerialMedium::serialize emits — no leading flag.
        let buf = [SERIAL_REVISION, 0x03, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, END_FLAG];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), Some(8));
    }

    #[test]
    fn frame_complete_with_leading_flag_skips_and_finds_end_flag() {
        // Forward-defensive: some DSP0253 variants emit a leading START
        // flag. frame_complete should skip the leading 0x7E and find the
        // trailing one, returning the FULL length (including leading flag).
        let buf = [END_FLAG, SERIAL_REVISION, 0x02, 0xAA, 0xBB, 0xCC, 0xDD, END_FLAG];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), Some(8));
    }

    #[test]
    fn frame_complete_extra_bytes_after_end_flag_returns_first_frame_len() {
        // First frame is 6 bytes (incl. END flag); 2 trailing bytes belong
        // to the next frame (caller's problem).
        let buf = [SERIAL_REVISION, 0x01, 0xAA, 0xBB, 0xCC, END_FLAG, 0x99, 0x88];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), Some(6));
    }

    #[test]
    fn frame_complete_back_to_back_after_leading_flag() {
        // Edge case: leading flag, then payload, then end flag, then more
        // bytes. Should return length of first frame INCLUDING the leading
        // flag; trailing bytes belong to a subsequent frame.
        let buf = [END_FLAG, SERIAL_REVISION, 0x01, 0xAA, 0xBB, END_FLAG, 0x77];
        assert_eq!(MctpSerialMedium.frame_complete(&buf).unwrap(), Some(6));
    }
}
