use crate::{
    MctpMessageBuffer, MctpPacketError,
    buffer_encoding::{DecodeError, EncodingDecoder},
    error::MctpPacketResult,
    mctp_transport_header::MctpTransportHeader,
    medium::MctpMedium,
};

pub(crate) fn map_decode_err<M: MctpMedium>(
    e: DecodeError,
    on_premature: &'static str,
    on_escape: &'static str,
) -> MctpPacketError<M> {
    match e {
        DecodeError::PrematureEnd => MctpPacketError::HeaderParseError(on_premature),
        DecodeError::InvalidEscape => MctpPacketError::HeaderParseError(on_escape),
    }
}

pub(crate) fn parse_transport_header<M: MctpMedium>(
    decoder: &mut EncodingDecoder<'_, M::Encoding>,
) -> MctpPacketResult<MctpTransportHeader, M> {
    // Read 4 decoded bytes through the encoding-aware decoder. We do NOT
    // pre-check `decoder.remaining_wire() < 4` because for stuffing
    // encodings wire length is not decoded length; PrematureEnd from
    // `read()` is the canonical "ran out of bytes while decoding the
    // header" signal — it correctly handles BOTH the Passthrough case
    // (wire < 4) AND the stuffing case (wire >= 4 but yields < 4 decoded
    // bytes).
    let mut header_bytes = [0u8; 4];
    for slot in header_bytes.iter_mut() {
        *slot = decoder.read().map_err(|e| {
            map_decode_err::<M>(
                e,
                "Packet is too small, cannot parse transport header",
                "Invalid encoding escape sequence in transport header",
            )
        })?;
    }
    let transport_header_value = u32::from_be_bytes(header_bytes);
    MctpTransportHeader::try_from(transport_header_value)
        .map_err(|_| MctpPacketError::HeaderParseError("Invalid transport header"))
}

pub(crate) fn parse_message_body<M: MctpMedium>(
    packet: &[u8],
) -> MctpPacketResult<(MctpMessageBuffer<'_>, Option<u8>), M> {
    // first four bytes are the message header, parse with MctpMessageHeader
    // to figure out the type, then based on that, parse the type specific header
    if packet.is_empty() {
        return Err(MctpPacketError::HeaderParseError(
            "packet too small to extract message type from header",
        ));
    }

    let integrity_check = packet[0] & 0b1000_0000;
    let message_type = packet[0] & 0b0111_1111;
    let packet = &packet[1..];

    // TODO - compute message integrity check if header.integrity_check is set
    Ok((
        MctpMessageBuffer {
            integrity_check,
            message_type,
            rest: packet,
        },
        None,
    ))
}
