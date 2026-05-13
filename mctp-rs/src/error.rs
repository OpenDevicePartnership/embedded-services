use crate::{
    endpoint_id::EndpointId, mctp_completion_code::MctpCompletionCode,
    mctp_message_tag::MctpMessageTag, mctp_sequence_number::MctpSequenceNumber, medium::MctpMedium,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ProtocolError {
    #[error("Expected start of message")]
    ExpectedStartOfMessage,
    #[error("Unexpected start of message")]
    UnexpectedStartOfMessage,
    #[error("Message tag mismatch")]
    MessageTagMismatch(MctpMessageTag, MctpMessageTag),
    #[error("Tag owner mismatch")]
    TagOwnerMismatch(u8, u8),
    #[error("Source endpoint id mismatch")]
    SourceEndpointIdMismatch(EndpointId, EndpointId),
    #[error("Unexpected packet sequence number")]
    UnexpectedPacketSequenceNumber(MctpSequenceNumber, MctpSequenceNumber),
    #[error("Received non-success completion code on request message")]
    CompletionCodeOnRequestMessage(MctpCompletionCode),
    #[error("Cannot send message while assembling")]
    SendMessageWhileAssembling,
    #[error("Cannot send message while receiving")]
    SendingMessageWhileReceiving,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpPacketError<M: MctpMedium> {
    HeaderParseError(&'static str),
    CommandParseError(&'static str),
    SerializeError(&'static str),
    UnsupportedMessageType(u8),
    ProtocolError(#[from] ProtocolError),
    MediumError(M::Error),
}

// TODO - MctpPacketResult type alias
pub type MctpPacketResult<T, Medium> = Result<T, MctpPacketError<Medium>>;
