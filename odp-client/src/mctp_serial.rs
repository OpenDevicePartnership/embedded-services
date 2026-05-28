use crate::{ODP_MESSAGE_TYPE, OdpError, OdpTransport};
use embedded_io::{Read, ReadExactError, Write};
use mctp_rs::{
    EndpointId, MctpMedium, MctpMessageHeaderTrait, MctpMessageTag, MctpMessageTrait, MctpPacketContext,
    MctpPacketError, MctpPacketResult, MctpReplyContext, MctpSequenceNumber, MctpSerialMedium,
};

/// MCTP serial framing END flag (DSP0253 0x7E).
const SERIAL_END_FLAG: u8 = 0x7E;

/// Inner buffer sizes — match the legacy `ec_relay.rs` which has been
/// proven to fit single-packet ODP messages.
const ASSEMBLY_BUF_LEN: usize = 128;
const RX_FRAME_BUF_LEN: usize = 128;

struct RawHeader([u8; 4]);

impl MctpMessageHeaderTrait for RawHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        if buffer.len() < 4 {
            return Err(MctpPacketError::SerializeError("buffer too small for odp header"));
        }
        buffer[..4].copy_from_slice(&self.0);
        Ok(4)
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        if buffer.len() < 4 {
            return Err(MctpPacketError::HeaderParseError("buffer too small for odp header"));
        }
        let mut h = [0u8; 4];
        h.copy_from_slice(&buffer[..4]);
        Ok((RawHeader(h), &buffer[4..]))
    }
}

struct RawBody<'b>(&'b [u8]);

impl<'buf> MctpMessageTrait<'buf> for RawBody<'buf> {
    type Header = RawHeader;
    const MESSAGE_TYPE: u8 = ODP_MESSAGE_TYPE;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        let n = self.0.len();
        if buffer.len() < n {
            return Err(MctpPacketError::SerializeError("buffer too small for odp body"));
        }
        buffer[..n].copy_from_slice(self.0);
        Ok(n)
    }

    fn deserialize<M: MctpMedium>(_h: &RawHeader, buffer: &'buf [u8]) -> MctpPacketResult<Self, M> {
        Ok(RawBody(buffer))
    }
}

fn reply_context(src: EndpointId, dst: EndpointId) -> MctpReplyContext<MctpSerialMedium> {
    MctpReplyContext {
        source_endpoint_id: src,
        destination_endpoint_id: dst,
        packet_sequence_number: MctpSequenceNumber::new(0),
        message_tag: MctpMessageTag::try_from(0).expect("tag 0 always valid"),
        medium_context: (),
    }
}

/// MCTP-over-serial implementation of [`OdpTransport`].
///
/// Owns the UART and an internal MCTP packet-assembly buffer. The MCTP
/// framing (per DSP0253: revision byte, byte_count, body, FCS-16, end
/// flag 0x7E) is encapsulated entirely inside this type — callers hand
/// in raw ODP wire bytes (4-byte header + body) and receive raw ODP
/// wire bytes back.
///
/// # Endpoint IDs
///
/// The transport is configured at construction time with `src_eid` and
/// `dst_eid` used in the MCTP header of outbound frames. Inbound frames
/// are MCTP-decoded but the EIDs are not validated against expectation
/// (the serial medium has no addressing of its own; the loopback in
/// tests and production callers tolerate whichever EID pair the peer
/// returns).
pub struct MctpSerialTransport<U: Read + Write> {
    uart: U,
    src_eid: EndpointId,
    dst_eid: EndpointId,
    rx_frame_buf: [u8; RX_FRAME_BUF_LEN],
    assembly_buf: [u8; ASSEMBLY_BUF_LEN],
}

impl<U: Read + Write> MctpSerialTransport<U> {
    /// Create a new transport wrapping `uart`, using `src_eid` / `dst_eid`
    /// in the MCTP header of every outbound frame. All MCTP framing
    /// (DSP0253 serial encapsulation) is handled internally; callers only
    /// see raw ODP wire bytes.
    pub fn new(uart: U, src_eid: EndpointId, dst_eid: EndpointId) -> Self {
        Self {
            uart,
            src_eid,
            dst_eid,
            rx_frame_buf: [0u8; RX_FRAME_BUF_LEN],
            assembly_buf: [0u8; ASSEMBLY_BUF_LEN],
        }
    }
}

impl<U: Read + Write> OdpTransport for MctpSerialTransport<U> {
    fn send_message(&mut self, payload: &[u8]) -> Result<(), OdpError> {
        if payload.len() < 4 {
            return Err(OdpError::BufferTooSmall);
        }
        let mut header = [0u8; 4];
        header.copy_from_slice(&payload[..4]);
        let body = &payload[4..];

        // Use a local tx buffer so `assembly_buf` stays free for recv.
        let mut tx_buf = [0u8; ASSEMBLY_BUF_LEN];
        let mut tx_ctx = MctpPacketContext::<MctpSerialMedium>::new(MctpSerialMedium, &mut tx_buf);
        let reply_ctx = reply_context(self.src_eid, self.dst_eid);
        let mut state = tx_ctx
            .serialize_packet(reply_ctx, (RawHeader(header), RawBody(body)))
            .map_err(|_| OdpError::Transport)?;
        while let Some(pkt) = state.next() {
            let pkt = pkt.map_err(|_| OdpError::Transport)?;
            self.uart.write_all(pkt).map_err(|_| OdpError::Transport)?;
        }
        Ok(())
    }

    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError> {
        // 1. Read serial frame (bytes until 0x7E) into rx_frame_buf.
        let mut filled = 0usize;
        loop {
            if filled >= self.rx_frame_buf.len() {
                return Err(OdpError::BufferTooSmall);
            }
            let mut byte = [0u8; 1];
            match self.uart.read_exact(&mut byte) {
                Ok(()) => {}
                Err(ReadExactError::UnexpectedEof) => return Err(OdpError::Transport),
                Err(ReadExactError::Other(_)) => return Err(OdpError::Transport),
            }
            self.rx_frame_buf[filled] = byte[0];
            filled += 1;
            if byte[0] == SERIAL_END_FLAG {
                break;
            }
        }

        // 2. MCTP-strip via a fresh PacketContext borrowing assembly_buf.
        let mut rx_ctx = MctpPacketContext::<MctpSerialMedium>::new(MctpSerialMedium, &mut self.assembly_buf);
        let message = rx_ctx
            .deserialize_packet(&self.rx_frame_buf[..filled])
            .map_err(|_| OdpError::Decode)?
            .ok_or(OdpError::Decode)?;

        // 3. Copy out the MCTP body (= ODP header+body wire bytes).
        let body = message.message_buffer.body();
        if buf.len() < body.len() {
            return Err(OdpError::BufferTooSmall);
        }
        buf[..body.len()].copy_from_slice(body);
        Ok(body.len())
    }
}
