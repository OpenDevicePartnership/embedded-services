use crate::{OdpError, OdpHeader, OdpTransport};

/// Decoded ODP response returned by [`Relay::invoke`].
///
/// The body slice borrows from the relay's internal assembly buffer; the
/// caller must finish reading the response before calling `invoke` again or
/// dropping the relay.
#[derive(Debug)]
pub struct OdpResponse<'a> {
    pub header: OdpHeader,
    pub body: &'a [u8],
}

/// Transport-blind abstraction over "issue a request and receive a
/// decoded response". Per-service code depends on `R: Relay` (or
/// `&mut dyn Relay`) rather than a concrete [`OdpClient<T>`].
///
/// The returned [`OdpResponse`] borrows from the relay's internal
/// buffer, so the borrow checker prevents calling `invoke` again or
/// dropping the relay until the response is consumed — no copy is required.
pub trait Relay {
    /// Encode `header` + `body`, send via the underlying transport,
    /// receive the response, validate the `message_id` round-trip, and
    /// return the decoded [`OdpResponse`].
    fn invoke<'a>(&'a mut self, header: OdpHeader, body: &[u8]) -> Result<OdpResponse<'a>, OdpError>;
}

/// Sync ODP client. Owns the transport and a 256-byte buffer used
/// sequentially for request encoding and response decoding.
pub struct OdpClient<T: OdpTransport> {
    transport: T,
    buf: [u8; 256],
}

impl<T: OdpTransport> OdpClient<T> {
    /// Create a new client, consuming `transport` by value.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            buf: [0u8; 256],
        }
    }
}

impl<T: OdpTransport> Relay for OdpClient<T> {
    fn invoke<'a>(&'a mut self, header: OdpHeader, body: &[u8]) -> Result<OdpResponse<'a>, OdpError> {
        let need = 4 + body.len();
        if need > self.buf.len() {
            return Err(OdpError::BufferTooSmall);
        }

        // Compose request: 4-byte BE header + body.
        self.buf[..4].copy_from_slice(&header.to_be_bytes());
        self.buf[4..need].copy_from_slice(body);
        self.transport.send_message(&self.buf[..need])?;

        // Receive response into the same buffer (send is complete).
        let n = self.transport.recv_message(&mut self.buf)?;
        if n < 4 {
            return Err(OdpError::Decode);
        }
        let mut hb = [0u8; 4];
        hb.copy_from_slice(&self.buf[..4]);
        let resp_header = OdpHeader::from_be_bytes(hb).map_err(|_| OdpError::Decode)?;

        if resp_header.message_id != header.message_id {
            return Err(OdpError::UnexpectedMessageId {
                expected: header.message_id,
                got: resp_header.message_id,
            });
        }

        Ok(OdpResponse {
            header: resp_header,
            body: &self.buf[4..n],
        })
    }
}
