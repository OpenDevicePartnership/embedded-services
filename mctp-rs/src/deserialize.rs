use crate::{
    MctpMessageBuffer, MctpPacketError, error::MctpPacketResult,
    mctp_transport_header::MctpTransportHeader, medium::MctpMedium,
};

pub(crate) fn parse_transport_header<M: MctpMedium>(
    packet: &[u8],
) -> MctpPacketResult<(MctpTransportHeader, &[u8]), M> {
    if packet.len() < 4 {
        return Err(MctpPacketError::HeaderParseError(
            "Packet is too small, cannot parse transport header",
        ));
    }
    let transport_header_value = u32::from_be_bytes(packet[0..4].try_into().map_err(|_| {
        MctpPacketError::HeaderParseError("Packet is too small, cannot parse transport header")
    })?);
    let transport_header = MctpTransportHeader::try_from(transport_header_value)
        .map_err(|_| MctpPacketError::HeaderParseError("Invalid transport header"))?;
    let packet = &packet[4..];
    Ok((transport_header, packet))
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
