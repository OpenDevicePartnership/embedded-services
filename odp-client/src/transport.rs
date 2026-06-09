use crate::OdpError;

/// Carries fully-framed ODP messages over an arbitrary medium.
///
/// Implementations own the underlying medium and any framing. Callers
/// hand in raw ODP wire bytes (header + body) and receive raw ODP wire
/// bytes back — they do not need to know what is underneath. This is the
/// seam that lets transports be swapped without rewriting callers.
pub trait OdpTransport {
    /// Send one fully-framed ODP message (header + body, wire bytes).
    fn send_message(&mut self, payload: &[u8]) -> Result<(), OdpError>;

    /// Receive one fully-framed ODP message into `buf`. Returns the number
    /// of bytes written, or [`OdpError::BufferTooSmall`] if `buf` cannot
    /// hold the entire message.
    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError>;
}
