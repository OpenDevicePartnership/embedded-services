/// Errors surfaced by the `odp-client` API.
///
/// Transport implementations map their own lower-level errors into these
/// variants so callers see one consistent surface.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OdpError {
    /// Underlying transport reported a failure.
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
                defmt::write!(
                    f,
                    "OdpError::UnexpectedMessageId {{ expected: {}, got: {} }}",
                    expected,
                    got
                )
            }
            OdpError::Decode => defmt::write!(f, "OdpError::Decode"),
        }
    }
}
