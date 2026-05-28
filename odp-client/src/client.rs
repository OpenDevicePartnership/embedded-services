use crate::{OdpError, OdpHeader, OdpTransport};

/// Decoded ODP response handed to the parse closure inside [`Relay::invoke`].
///
/// The body slice borrows from the client's internal assembly buffer; only
/// owned values can escape the closure.
pub struct OdpResponse<'a> {
    pub header: OdpHeader,
    pub body: &'a [u8],
}

/// Transport-blind abstraction over "issue a request to the EC and parse the
/// response". Per-service code depends on `R: Relay`, never on a concrete
/// `OdpClient<T>`.
///
/// The closure pattern is required because the response body slice borrows
/// from the client's internal buffer; parsing inside the closure guarantees
/// only owned types escape.
///
/// # Object safety
///
/// `invoke` is generic over `R` (return type) and `F` (closure type), so
/// `Relay` is **not** object-safe. Use it as a bound (`impl Relay` /
/// `R: Relay`), not as `dyn Relay`.
pub trait Relay {
    /// Encode `header` + `body` as an ODP wire frame, hand them to the
    /// underlying transport, receive the response, validate the
    /// `message_id` round-trip, and pass an [`OdpResponse`] to `parse`.
    ///
    /// The parse closure runs inside `invoke` so that the borrow of the
    /// client's internal buffer is confined to the closure scope; any
    /// owned value returned from the closure escapes cleanly.
    fn invoke<R, F>(
        &mut self,
        header: OdpHeader,
        body: &[u8],
        parse: F,
    ) -> Result<R, OdpError>
    where
        F: FnOnce(OdpResponse<'_>) -> Result<R, OdpError>;
}

/// Sync ODP-over-transport client.
///
/// Owns the transport and a 256-byte assembly buffer used for both request
/// encoding and response decoding (sequentially — the phases never overlap
/// because send completes before recv begins).
pub struct OdpClient<T: OdpTransport> {
    transport: T,
    buf: [u8; 256],
}

impl<T: OdpTransport> OdpClient<T> {
    /// Create a new client, consuming `transport` by value.
    pub fn new(transport: T) -> Self {
        Self { transport, buf: [0u8; 256] }
    }
}

impl<T: OdpTransport> Relay for OdpClient<T> {
    fn invoke<R, F>(
        &mut self,
        header: OdpHeader,
        body: &[u8],
        parse: F,
    ) -> Result<R, OdpError>
    where
        F: FnOnce(OdpResponse<'_>) -> Result<R, OdpError>,
    {
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

        parse(OdpResponse { header: resp_header, body: &self.buf[4..n] })
    }
}
