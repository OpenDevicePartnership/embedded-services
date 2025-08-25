use core::ops::{Div, Mul};

/// Append an MCTP header to the front of a message.
/// Returns the message and its new total with the appended header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpError {
    /// Header is not at least 9 bytes long.
    InvalidHeaderSize,
    /// Wrong destination address.
    WrongDestinationAddr,
    /// Invalid command code.
    InvalidCommandCode,
    /// Invalid byte count, encoded byte count does not match MCTP message length.
    InvalidByteCount,
    /// Invalid header version. Should be 1.
    InvalidHeaderVersion,
    /// Invalid destination endpoint
    InvalidDestinationEndpoint,
    /// Invalid source endpoint.
    InvalidSourceEndpoint,
    /// Multi message not supported.
    InvalidFlags,
}

/// Data type for MCTP message underlying data size.
pub type PayloadLen = usize;

/// Max payload len, due to SMBUS block transaction limits.
pub const MAX_MCTP_BYTE_COUNT: usize = 69;
/// Payload len + bytes for destination target address, command code, and byte count.
pub const MAX_MCTP_PACKET_LEN: usize = MAX_MCTP_BYTE_COUNT + 3;

fn round_up_to_nearest_mod_4(unrounded: usize) -> usize {
    unrounded + (unrounded % 4)
}

/// Decode a header from and MCTP message.
/// Returns the underlying data and its service endpoint ID and the underlying data size.
pub fn handle_mctp_header(
    mctp_msg: &[u8],
    data: &mut [u8],
) -> Result<(crate::comms::EndpointID, PayloadLen), MctpError> {
    // assert we have at least 9 bytes, minimum
    if mctp_msg.len() < 9 {
        return Err(MctpError::InvalidHeaderSize);
    }

    // EC is at address 2, if we have anything other than 2 reject it.
    if mctp_msg[0] != 2 {
        return Err(MctpError::WrongDestinationAddr);
    }

    // MCTP command code is 0x0F.
    if mctp_msg[1] != 0x0F {
        return Err(MctpError::InvalidCommandCode);
    }

    // Check the byte count is correctly formed and is not larger than the max in the spec.
    if usize::from(mctp_msg[2]) > MAX_MCTP_BYTE_COUNT {
        return Err(MctpError::InvalidByteCount);
    }
    // Some eSPI controllers behave oddly if packet sizes aren't multiples of 4, so the MCTP message is padded
    // to multiples of 4.
    // Byte size + header size (3) + padding to align size to multiple of 4 should equal length of message.
    // Unfortunately since padding is variable, there is no way to validate byte count is truly correct.
    // There is a chance that the number of valid bytes exceeds the byte count if mctp_msg.len()
    // is not a multiple of 4 (and thus has padding bytes)
    if ((usize::from(mctp_msg[2]) + 3) + 3).div(4).mul(4) != mctp_msg.len() {
        return Err(MctpError::InvalidByteCount);
    }

    // Only support header version 1.
    if mctp_msg[4] != 1 {
        return Err(MctpError::InvalidHeaderVersion);
    }

    // Only subsystem supported currently is battery (0x02) and thermal (0x03).
    let endpoint_id = match mctp_msg[5] {
        2 => crate::comms::EndpointID::Internal(crate::comms::Internal::Battery),
        3 => crate::comms::EndpointID::Internal(crate::comms::Internal::Thermal),
        _ => return Err(MctpError::InvalidDestinationEndpoint),
    };

    // Only source endpoint supported currently is host (1).
    if mctp_msg[6] != 1 {
        return Err(MctpError::InvalidSourceEndpoint);
    }

    let som = mctp_msg[7] & (1 << 7) != 0;
    let eom = mctp_msg[7] & (1 << 6) != 0;
    let seq_num = (mctp_msg[7] & 0b0011_0000) >> 4;
    let msg_tag = mctp_msg[7] & 0b0000_0111;

    // Verify flags
    if !som || !eom || seq_num != 1 || msg_tag != 3 {
        return Err(MctpError::InvalidFlags);
    }

    let len = usize::from(mctp_msg[2]) - 5;
    // Copy message contents without the padding to a multiple of 4 at the end.
    data[..len].copy_from_slice(&mctp_msg[8..8 + len]);

    Ok((endpoint_id, len))
}

/// Append an MCTP header to the front of a message.
/// Returns the message and its new total with the appended header.
pub fn build_mctp_header(
    data: &[u8],
    data_len: usize,
    src_endpoint: crate::comms::EndpointID,
    start_of_msg: bool,
    end_of_msg: bool,
) -> Result<([u8; MAX_MCTP_PACKET_LEN], usize), MctpError> {
    let mut ret = [0u8; MAX_MCTP_PACKET_LEN];
    let padding = [0u8; 3];

    // Host is at address 0.
    ret[0] = 0;

    // MCTP command code is 0x0F.
    ret[1] = 0x0F;

    // Size of the payload length + header size, without padding
    ret[2] = (data_len + 5) as u8;

    // Source is EC (upper 7 bits = 0x01 | hardcoded LSB of 0x01)
    ret[3] = 3;

    // Header version is 1
    ret[4] = 1;

    // Destination endpoint ID is Host (0x01)
    ret[5] = 1;

    // Only subsystem supported currently is battery (0x02) and thermal (0x03).
    match src_endpoint {
        crate::comms::EndpointID::Internal(crate::comms::Internal::Battery) => ret[6] = 2,
        crate::comms::EndpointID::Internal(crate::comms::Internal::Thermal) => ret[6] = 3,
        _ => return Err(MctpError::InvalidDestinationEndpoint),
    }

    // Seq num 1 + Msg tag 3
    ret[7] = 0x13;
    if start_of_msg {
        ret[7] |= 1 << 7;
    }
    if end_of_msg {
        ret[7] |= 1 << 6;
    }

    // True packet size must be a multple of 4. Header is 8 bytes which is already a multiple of 4,
    // so we don't need to include it here.
    let data_len_padded = round_up_to_nearest_mod_4(data_len);

    ret[8..data_len + 8].copy_from_slice(&data[..data_len]);

    // Add padding to align to 4 bytes
    ret[data_len + 8..data_len_padded + 8].copy_from_slice(&padding[..data_len_padded - data_len]);

    Ok((ret, data_len_padded + 8))
}
