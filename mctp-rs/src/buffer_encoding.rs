//! Stateless byte-level buffer-encoding transform for MCTP media.
//!
//! Most media (SMBus/eSPI) ship MCTP packets verbatim — wire bytes ARE
//! payload bytes. Some media (DSP0253 serial) need byte-stuffing: an
//! escape character expands certain payload bytes into 2-byte sequences
//! on the wire, and decode reverses that transform.
//!
//! [`BufferEncoding`] is the byte-stuffing layer ONLY. It is stateless:
//! [`write_byte`](BufferEncoding::write_byte) and
//! [`read_byte`](BufferEncoding::read_byte) are associated functions with
//! no `self` and no struct state. Higher-level framing concerns
//! (start/end delimiters, FCS / CRC) live on the medium type, not here.

use core::marker::PhantomData;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EncodeError {
    /// `wire_buf` did not have room for the encoded bytes (1 for plain,
    /// up to 2 for an escape sequence). The caller should advance no
    /// cursors and treat the encode as failed.
    BufferFull,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DecodeError {
    /// `wire_buf` was empty or ended mid-escape-sequence. Indicates the
    /// caller asked to decode past the end of valid wire data.
    PrematureEnd,
    /// An escape byte was followed by a byte not in the medium's
    /// accept-list (strict-XOR rule per RFC1662 §4.2 / DSP0253 §6.4).
    /// The caller should reject the entire frame. Currently unreachable;
    /// produced only by stuffing encodings introduced in a later phase.
    InvalidEscape,
}

/// Stateless byte-stuffing transform. Implementors define how a single
/// logical (payload) byte maps to one or more wire bytes (encode) and
/// how a wire-byte prefix maps back to a single payload byte (decode).
///
/// All methods are associated functions — there is no `self` and no
/// struct state. Callers own the buffers and the read/write cursors.
pub trait BufferEncoding {
    /// Encode one logical payload byte into `wire_buf` starting at
    /// index 0. Returns the number of wire bytes written (1 for plain,
    /// 2 for an escape sequence). The caller advances their write
    /// cursor by the returned count.
    fn write_byte(wire_buf: &mut [u8], byte: u8) -> Result<usize, EncodeError>;

    /// Decode the next logical payload byte from `wire_buf` starting at
    /// index 0. Returns `(decoded_byte, wire_bytes_consumed)`. The
    /// caller advances their read cursor by `wire_bytes_consumed`.
    fn read_byte(wire_buf: &[u8]) -> Result<(u8, usize), DecodeError>;

    /// Wire-byte footprint of `decoded` under this encoding. Must equal
    /// the sum of `write_byte(_, b)` lengths for each `b` in `decoded`.
    /// NO default impl: every encoding declares its sizing rule
    /// explicitly.
    fn wire_size_of(decoded: &[u8]) -> usize;
}

/// No-op encoding: wire bytes ARE payload bytes. Used by media that do
/// not byte-stuff (SMBus/eSPI, test fixtures).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PassthroughEncoding;

impl BufferEncoding for PassthroughEncoding {
    fn write_byte(wire_buf: &mut [u8], byte: u8) -> Result<usize, EncodeError> {
        match wire_buf.first_mut() {
            Some(slot) => {
                *slot = byte;
                Ok(1)
            }
            None => Err(EncodeError::BufferFull),
        }
    }

    fn read_byte(wire_buf: &[u8]) -> Result<(u8, usize), DecodeError> {
        match wire_buf.first() {
            Some(&byte) => Ok((byte, 1)),
            None => Err(DecodeError::PrematureEnd),
        }
    }

    fn wire_size_of(decoded: &[u8]) -> usize {
        decoded.len()
    }
}

/// Stateful cursor over a `&[u8]` wire buffer that reads decoded bytes
/// through `E: BufferEncoding`. Constructed by [`MctpMedium::deserialize`]
/// and handed to higher layers so they cannot bypass the encoding by
/// slicing the underlying buffer directly.
///
/// [`MctpMedium::deserialize`]: crate::medium::MctpMedium::deserialize
pub struct EncodingDecoder<'buf, E: BufferEncoding> {
    buf: &'buf [u8],
    wire_pos: usize,
    _phantom: PhantomData<E>,
}

impl<'buf, E: BufferEncoding> EncodingDecoder<'buf, E> {
    /// Wrap a wire-byte buffer for stateful encoding-mediated reads.
    pub fn new(buf: &'buf [u8]) -> Self {
        Self {
            buf,
            wire_pos: 0,
            _phantom: PhantomData,
        }
    }

    /// Read one decoded byte. Advances the wire cursor by the encoding's
    /// per-byte wire footprint. Returns `DecodeError::PrematureEnd` when
    /// the wire buffer is exhausted (or ends mid-escape) and
    /// `DecodeError::InvalidEscape` for malformed escape sequences.
    pub fn read(&mut self) -> Result<u8, DecodeError> {
        let (byte, n) = E::read_byte(&self.buf[self.wire_pos..])?;
        self.wire_pos += n;
        Ok(byte)
    }
}

/// Stateful cursor over a `&mut [u8]` wire buffer that writes decoded
/// bytes through `E: BufferEncoding`. Constructed by
/// [`MctpMedium::serialize`] and handed to the caller's `message_writer`
/// closure so the closure cannot bypass the encoding.
///
/// [`MctpMedium::serialize`]: crate::medium::MctpMedium::serialize
pub struct EncodingEncoder<'buf, E: BufferEncoding> {
    buf: &'buf mut [u8],
    wire_pos: usize,
    _phantom: PhantomData<E>,
}

impl<'buf, E: BufferEncoding> EncodingEncoder<'buf, E> {
    /// Wrap a wire-byte buffer for stateful encoding-mediated writes.
    pub fn new(buf: &'buf mut [u8]) -> Self {
        Self {
            buf,
            wire_pos: 0,
            _phantom: PhantomData,
        }
    }

    /// Write one decoded byte. Advances the wire cursor by the encoding's
    /// per-byte wire footprint. Returns `EncodeError::BufferFull` when
    /// the underlying wire buffer cannot fit the encoded representation.
    pub fn write(&mut self, byte: u8) -> Result<(), EncodeError> {
        let n = E::write_byte(&mut self.buf[self.wire_pos..], byte)?;
        self.wire_pos += n;
        Ok(())
    }

    /// Write a contiguous slice of decoded bytes; aborts on the first
    /// encode error. Equivalent to a `for &b in bytes { self.write(b)? }`
    /// loop, but more concise at call sites that just splat a byte slice.
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        for &b in bytes {
            self.write(b)?;
        }
        Ok(())
    }

    /// Wire bytes written so far (the size of the produced wire frame).
    pub fn wire_position(&self) -> usize {
        self.wire_pos
    }

    /// Wire bytes remaining in the underlying buffer.
    pub fn remaining_wire(&self) -> usize {
        self.buf.len() - self.wire_pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_write_byte_writes_one_byte() {
        let mut buf = [0u8; 4];
        let n = PassthroughEncoding::write_byte(&mut buf, 0xAB).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf, [0xAB, 0, 0, 0]);
    }

    #[test]
    fn passthrough_write_byte_full_buffer() {
        let mut buf = [];
        let err = PassthroughEncoding::write_byte(&mut buf, 0xAB).unwrap_err();
        assert_eq!(err, EncodeError::BufferFull);
    }

    #[test]
    fn passthrough_read_byte_reads_one_byte() {
        let buf = [0xAB, 0xCD];
        let (b, n) = PassthroughEncoding::read_byte(&buf).unwrap();
        assert_eq!(b, 0xAB);
        assert_eq!(n, 1);
    }

    #[test]
    fn passthrough_read_byte_premature_end() {
        let buf = [];
        let err = PassthroughEncoding::read_byte(&buf).unwrap_err();
        assert_eq!(err, DecodeError::PrematureEnd);
    }

    #[test]
    fn decoder_reads_all_bytes_via_passthrough() {
        let buf = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut decoder = EncodingDecoder::<PassthroughEncoding>::new(&buf);
        assert_eq!(decoder.read().unwrap(), 0xAA);
        assert_eq!(decoder.read().unwrap(), 0xBB);
        assert_eq!(decoder.read().unwrap(), 0xCC);
        assert_eq!(decoder.read().unwrap(), 0xDD);
        assert_eq!(decoder.read().unwrap_err(), DecodeError::PrematureEnd);
    }

    #[test]
    fn encoder_writes_all_bytes_via_passthrough() {
        let mut buf = [0u8; 4];
        {
            let mut encoder = EncodingEncoder::<PassthroughEncoding>::new(&mut buf);
            assert_eq!(encoder.wire_position(), 0);
            assert_eq!(encoder.remaining_wire(), 4);
            encoder.write(0x11).unwrap();
            encoder.write(0x22).unwrap();
            encoder.write(0x33).unwrap();
            encoder.write(0x44).unwrap();
            assert_eq!(encoder.wire_position(), 4);
            assert_eq!(encoder.remaining_wire(), 0);
            assert_eq!(encoder.write(0x55).unwrap_err(), EncodeError::BufferFull);
        }
        assert_eq!(buf, [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn passthrough_wire_size_of_returns_input_len() {
        assert_eq!(PassthroughEncoding::wire_size_of(&[]), 0);
        assert_eq!(PassthroughEncoding::wire_size_of(&[0xAB]), 1);
        let buf = [0u8; 64];
        assert_eq!(PassthroughEncoding::wire_size_of(&buf), 64);
    }
}
