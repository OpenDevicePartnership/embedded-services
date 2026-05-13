use crate::{
    MctpMessage, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError,
    deserialize::{map_decode_err, parse_message_body, parse_transport_header},
    endpoint_id::EndpointId,
    error::{MctpPacketResult, ProtocolError},
    mctp_message_tag::MctpMessageTag,
    mctp_sequence_number::MctpSequenceNumber,
    medium::{MctpMedium, MctpMediumFrame},
    serialize::SerializePacketState,
};

/// Represents the state needed to construct a repsonse to a request:
/// the MCTP transport source/destination, the sequence number to use for
/// the reply, and the medium-specific context that came with the request.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpReplyContext<M: MctpMedium> {
    pub destination_endpoint_id: EndpointId,
    pub source_endpoint_id: EndpointId,
    pub packet_sequence_number: MctpSequenceNumber,
    pub message_tag: MctpMessageTag,
    pub medium_context: M::ReplyContext,
}

/// Context for serializing and deserializing an MCTP message, which may be split among multiple
/// packets.
pub struct MctpPacketContext<'buf, M: MctpMedium> {
    assembly_state: AssemblyState,
    medium: M,
    packet_assembly_buffer: &'buf mut [u8],
}

impl<'buf, M: MctpMedium> MctpPacketContext<'buf, M> {
    pub fn new(medium: M, packet_assembly_buffer: &'buf mut [u8]) -> Self {
        Self {
            medium,
            assembly_state: AssemblyState::Idle,
            packet_assembly_buffer,
        }
    }

    pub fn deserialize_packet(&mut self, packet: &[u8]) -> MctpPacketResult<Option<MctpMessage<'_, M>>, M> {
        let (medium_frame, mut decoder) = self.medium.deserialize(packet)?;
        let transport_header = parse_transport_header::<M>(&mut decoder)?;

        let mut state = match self.assembly_state {
            AssemblyState::Idle => {
                if transport_header.start_of_message == 0 {
                    return Err(MctpPacketError::ProtocolError(ProtocolError::ExpectedStartOfMessage));
                }

                AssemblingState {
                    message_tag: transport_header.message_tag,
                    tag_owner: transport_header.tag_owner,
                    source_endpoint_id: transport_header.source_endpoint_id,
                    packet_sequence_number: transport_header.packet_sequence_number,
                    packet_assembly_buffer_index: 0,
                }
            }
            AssemblyState::Receiving(assembling_state) => {
                if transport_header.start_of_message != 0 {
                    return Err(MctpPacketError::ProtocolError(ProtocolError::UnexpectedStartOfMessage));
                }
                if assembling_state.message_tag != transport_header.message_tag {
                    return Err(MctpPacketError::ProtocolError(ProtocolError::MessageTagMismatch(
                        assembling_state.message_tag,
                        transport_header.message_tag,
                    )));
                }
                if assembling_state.tag_owner != transport_header.tag_owner {
                    return Err(MctpPacketError::ProtocolError(ProtocolError::TagOwnerMismatch(
                        assembling_state.tag_owner,
                        transport_header.tag_owner,
                    )));
                }
                if assembling_state.source_endpoint_id != transport_header.source_endpoint_id {
                    return Err(MctpPacketError::ProtocolError(ProtocolError::SourceEndpointIdMismatch(
                        assembling_state.source_endpoint_id,
                        transport_header.source_endpoint_id,
                    )));
                }
                let expected_sequence_number = assembling_state.packet_sequence_number.next();
                if expected_sequence_number != transport_header.packet_sequence_number {
                    return Err(MctpPacketError::ProtocolError(
                        ProtocolError::UnexpectedPacketSequenceNumber(
                            expected_sequence_number,
                            transport_header.packet_sequence_number,
                        ),
                    ));
                }
                assembling_state
            }
        };

        let buffer_idx = state.packet_assembly_buffer_index;
        let packet_size = medium_frame.packet_size();
        if packet_size < 4 {
            return Err(MctpPacketError::HeaderParseError(
                "transport frame indicated packet length < 4",
            ));
        }
        let packet_size = packet_size - 4; // to account for the transport header
        // Check assembly buffer bounds (decoded bytes destination)
        if buffer_idx + packet_size > self.packet_assembly_buffer.len() {
            return Err(MctpPacketError::HeaderParseError(
                "packet assembly buffer overflow - insufficient space",
            ));
        }
        // Decode `packet_size` payload bytes from the (possibly stuffed) wire
        // buffer into the assembly buffer one byte at a time via the
        // medium-supplied decoder. We do NOT pre-check
        // `decoder.remaining_wire() < packet_size` because for stuffing
        // encodings wire length is not decoded length; PrematureEnd from
        // `read()` is the canonical "ran out of bytes while decoding the
        // body" signal.
        for i in 0..packet_size {
            self.packet_assembly_buffer[buffer_idx + i] = decoder.read().map_err(|e| {
                map_decode_err::<M>(
                    e,
                    "packet body too short to extract expected decoded bytes",
                    "Invalid encoding escape sequence in packet body",
                )
            })?;
        }
        state.packet_assembly_buffer_index += packet_size;

        let message = if transport_header.end_of_message == 1 {
            self.assembly_state = AssemblyState::Idle;
            let (message_body, message_integrity_check) =
                parse_message_body::<M>(&self.packet_assembly_buffer[..state.packet_assembly_buffer_index])?;
            Some(MctpMessage {
                reply_context: MctpReplyContext {
                    destination_endpoint_id: transport_header.destination_endpoint_id,
                    source_endpoint_id: transport_header.source_endpoint_id,
                    packet_sequence_number: transport_header.packet_sequence_number,
                    message_tag: transport_header.message_tag,
                    medium_context: medium_frame.reply_context(),
                },
                message_buffer: message_body,
                message_integrity_check,
            })
        } else {
            self.assembly_state = AssemblyState::Receiving(state);
            None
        };

        Ok(message)
    }

    pub fn serialize_packet<P: MctpMessageTrait<'buf>>(
        &'buf mut self,
        reply_context: MctpReplyContext<M>,
        message: (P::Header, P),
    ) -> MctpPacketResult<SerializePacketState<'buf, M>, M> {
        match self.assembly_state {
            AssemblyState::Idle => {}
            _ => {
                return Err(MctpPacketError::ProtocolError(
                    ProtocolError::SendMessageWhileAssembling,
                ));
            }
        };

        self.packet_assembly_buffer[0] = P::MESSAGE_TYPE;
        let header_size = message.0.serialize(&mut self.packet_assembly_buffer[1..])?;
        let body_size = message
            .1
            .serialize(&mut self.packet_assembly_buffer[header_size + 1..])?;

        let (message, rest) = self.packet_assembly_buffer.split_at_mut(header_size + body_size + 1);

        Ok(SerializePacketState {
            medium: &self.medium,
            reply_context,
            current_packet_num: 0,
            serialized_message_header: false,
            message_buffer: message,
            assembly_buffer: rest,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum AssemblyState {
    Idle,
    Receiving(AssemblingState),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct AssemblingState {
    message_tag: MctpMessageTag,
    tag_owner: u8,
    source_endpoint_id: EndpointId,
    packet_sequence_number: MctpSequenceNumber,
    packet_assembly_buffer_index: usize,
}
