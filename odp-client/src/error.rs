/// Errors surfaced by the `odp-client` API.
///
/// The transport layer maps lower-level MCTP / framing errors into these
/// variants so that callers see one consistent error surface and don't have
/// to depend on `mctp-rs` directly.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OdpError {
    /// Underlying transport (serial / MCTP framing) reported a failure.
    Transport,
    /// Timed out waiting for a response from the peer.
    Timeout,
    /// Caller-supplied buffer was too small to hold the message.
    BufferTooSmall,
    /// Peer responded with a different `OdpService` than the request.
    UnexpectedResponseKind,
    /// Peer responded with a different `message_id` than the request.
    UnexpectedMessageId { expected: u16, got: u16 },
    /// Failed to decode an incoming ODP message (malformed header / body).
    Decode,
}

#[cfg(feature = "defmt")]
impl defmt::Format for OdpError {
    fn format(&self, f: defmt::Formatter) {
        match self {
            OdpError::Transport => defmt::write!(f, "OdpError::Transport"),
            OdpError::Timeout => defmt::write!(f, "OdpError::Timeout"),
            OdpError::BufferTooSmall => defmt::write!(f, "OdpError::BufferTooSmall"),
            OdpError::UnexpectedResponseKind => defmt::write!(f, "OdpError::UnexpectedResponseKind"),
            OdpError::UnexpectedMessageId { expected, got } => {
                defmt::write!(f, "OdpError::UnexpectedMessageId {{ expected: {}, got: {} }}", expected, got)
            }
            OdpError::Decode => defmt::write!(f, "OdpError::Decode"),
        }
    }
}
