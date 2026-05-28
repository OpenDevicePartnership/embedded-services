use crate::OdpError;

/// Carries a single fully-framed ODP message over the wire.
///
/// Implementations encapsulate any MCTP framing internally. The public
/// surface is bytes only — callers do not need to know whether the
/// underlying transport is MCTP-over-serial, MCTP-over-eSPI, or anything
/// else. This is the seam that lets us swap transports without rewriting
/// callers.
pub trait OdpTransport {
    /// Send one fully-framed ODP message (header + body, wire bytes).
    ///
    /// Returns once the message has been handed to the transport.
    fn send_message(&mut self, payload: &[u8]) -> Result<(), OdpError>;

    /// Receive one fully-framed ODP message into `buf`.
    ///
    /// Returns the number of bytes written. Returns
    /// `OdpError::BufferTooSmall` if `buf` cannot hold the entire message.
    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError>;
}
