use crate::{
    MctpPacketError,
    buffer_encoding::{BufferEncoding, EncodeError, EncodingEncoder},
    error::MctpPacketResult,
    mctp_packet_context::MctpReplyContext,
    mctp_transport_header::MctpTransportHeader,
    medium::MctpMedium,
};

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SerializePacketState<'buf, M: MctpMedium> {
    pub(crate) medium: &'buf M,
    pub(crate) reply_context: MctpReplyContext<M>,
    pub(crate) current_packet_num: u8,
    pub(crate) serialized_message_header: bool,
    pub(crate) message_buffer: &'buf [u8],
    pub(crate) assembly_buffer: &'buf mut [u8],
}

pub const TRANSPORT_HEADER_SIZE: usize = 4;

impl<'buf, M: MctpMedium> SerializePacketState<'buf, M> {
    pub fn next(&mut self) -> Option<MctpPacketResult<&[u8], M>> {
        if self.message_buffer.is_empty() {
            return None;
        }

        let packet = self.medium.serialize(
            self.reply_context.medium_context,
            self.assembly_buffer,
            |encoder: &mut EncodingEncoder<'_, M::Encoding>| {
                let max_wire = self
                    .medium
                    .max_message_body_size()
                    .min(encoder.remaining_wire());

                // Build the transport header first (with end_of_message
                // tentatively 0) so we can measure its wire footprint
                // under the medium's encoding before chunking the body.
                let start_of_message = if self.current_packet_num == 0 { 1 } else { 0 };
                let packet_sequence_number = self.reply_context.packet_sequence_number.inc();
                let mut transport_header_value: u32 = MctpTransportHeader {
                    reserved: 0,
                    header_version: 1,
                    start_of_message,
                    end_of_message: 0,
                    packet_sequence_number,
                    tag_owner: 0,
                    message_tag: self.reply_context.message_tag,
                    source_endpoint_id: self.reply_context.destination_endpoint_id,
                    destination_endpoint_id: self.reply_context.source_endpoint_id,
                }
                .try_into()
                .map_err(MctpPacketError::SerializeError)?;
                let mut header_bytes = transport_header_value.to_be_bytes();
                let header_wire_cost = M::Encoding::wire_size_of(&header_bytes);
                if header_wire_cost > max_wire {
                    return Err(MctpPacketError::SerializeError(
                        "assembly buffer too small for mctp transport header",
                    ));
                }

                // Walk decoded body bytes one at a time, accumulating
                // their per-byte wire footprint via
                // `M::Encoding::wire_size_of`. Stop when adding the
                // next byte would exceed the wire budget. Correct for
                // both passthrough and stuffing encodings (both shipped
                // encodings are byte-additive — `wire_size_of(a ++ b)
                // == wire_size_of(a) + wire_size_of(b)`).
                let body_wire_budget = max_wire - header_wire_cost;
                let mut consumed_wire = 0usize;
                let mut message_size = 0usize;
                for &b in self.message_buffer.iter() {
                    let cost = M::Encoding::wire_size_of(&[b]);
                    if consumed_wire + cost > body_wire_budget {
                        break;
                    }
                    consumed_wire += cost;
                    message_size += 1;
                }

                // if there is no room for any of the body, and the body is not empty,
                // then return an error, otherwise we infinate loop sending packets with headers and
                // no body, making it impossible to ever assemble a message
                if message_size == 0 && !self.message_buffer.is_empty() {
                    return Err(MctpPacketError::SerializeError(
                        "assembly buffer too small for non-empty message body",
                    ));
                }

                let body = &self.message_buffer[..message_size];
                self.message_buffer = &self.message_buffer[message_size..];

                // Now that we know whether this is the final chunk,
                // rebuild the transport header if `end_of_message`
                // flips to 1. Re-measure the wire cost — none of the
                // EOM-bit bytes hit 0x7E or 0x7D under either shipped
                // encoding in practice, but do not assume.
                let end_of_message = if self.message_buffer.is_empty() { 1 } else { 0 };
                if end_of_message == 1 {
                    transport_header_value = MctpTransportHeader {
                        reserved: 0,
                        header_version: 1,
                        start_of_message,
                        end_of_message,
                        packet_sequence_number,
                        tag_owner: 0,
                        message_tag: self.reply_context.message_tag,
                        source_endpoint_id: self.reply_context.destination_endpoint_id,
                        destination_endpoint_id: self.reply_context.source_endpoint_id,
                    }
                    .try_into()
                    .map_err(MctpPacketError::SerializeError)?;
                    header_bytes = transport_header_value.to_be_bytes();
                    let rebuilt_header_wire_cost = M::Encoding::wire_size_of(&header_bytes);
                    if rebuilt_header_wire_cost + consumed_wire > max_wire {
                        return Err(MctpPacketError::SerializeError(
                            "assembly buffer too small after EOM bit set",
                        ));
                    }
                }

                // write the transport header and message body via the
                // medium-supplied encoder.
                let map_encode_err = |e: EncodeError| match e {
                    EncodeError::BufferFull => {
                        MctpPacketError::SerializeError("encoding: buffer full")
                    }
                };
                encoder.write_all(&header_bytes).map_err(map_encode_err)?;
                encoder.write_all(body).map_err(map_encode_err)?;
                Ok(())
            },
        );

        // Increment packet number for next call
        if packet.is_ok() {
            self.current_packet_num += 1;
        }

        Some(packet)
    }
}
