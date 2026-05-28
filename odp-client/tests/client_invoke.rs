use odp_client::{LoopbackTransport, OdpClient, OdpError, OdpHeader, OdpResponse, OdpService, Relay};

#[test]
fn invoke_round_trips_request_and_parses_response() {
    // LoopbackTransport echoes the request bytes back as the response.
    let transport = LoopbackTransport::new();
    let mut client = OdpClient::new(transport);

    let req_header = OdpHeader {
        is_request: true,
        service: OdpService::Battery,
        is_error: false,
        message_id: 0x11,
    };

    let parsed: u8 = client
        .invoke(req_header, &[0xAA, 0xBB], |resp: OdpResponse<'_>| {
            assert_eq!(resp.header.service, OdpService::Battery);
            assert_eq!(resp.header.message_id, 0x11);
            Ok(resp.body[0])
        })
        .unwrap();
    assert_eq!(parsed, 0xAA);
}

#[test]
fn invoke_with_oversized_body_returns_buffer_too_small() {
    // Internal buf is 256 bytes; 4-byte header + 253-byte body = 257 → overflow.
    let mut client = OdpClient::new(LoopbackTransport::new());
    let req_header = OdpHeader {
        is_request: true,
        service: OdpService::Battery,
        is_error: false,
        message_id: 1,
    };
    let huge = [0u8; 253];
    let err = client
        .invoke(req_header, &huge, |_| Ok::<(), OdpError>(()))
        .unwrap_err();
    assert_eq!(err, OdpError::BufferTooSmall);
}

/// Inline stub transport that allows pre-loading recv bytes independently
/// of what was sent. Used to test message_id mismatch detection.
struct StubTransport {
    recv_bytes: [u8; 64],
    recv_len: usize,
}

impl StubTransport {
    fn with_recv(bytes: &[u8]) -> Self {
        let mut buf = [0u8; 64];
        buf[..bytes.len()].copy_from_slice(bytes);
        Self { recv_bytes: buf, recv_len: bytes.len() }
    }
}

impl odp_client::OdpTransport for StubTransport {
    fn send_message(&mut self, _data: &[u8]) -> Result<(), OdpError> {
        Ok(())
    }
    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError> {
        let n = self.recv_len;
        buf[..n].copy_from_slice(&self.recv_bytes[..n]);
        Ok(n)
    }
}

#[test]
fn invoke_with_mismatched_response_message_id_returns_unexpected() {
    // Pre-load a response with message_id=0x99; request uses message_id=0x11.
    let stale_header = OdpHeader {
        is_request: false,
        service: OdpService::Battery,
        is_error: false,
        message_id: 0x99,
    };
    let recv_bytes = stale_header.to_be_bytes();
    let transport = StubTransport::with_recv(&recv_bytes);
    let mut client = OdpClient::new(transport);

    let req_header = OdpHeader {
        is_request: true,
        service: OdpService::Battery,
        is_error: false,
        message_id: 0x11,
    };
    let err = client
        .invoke(req_header, &[], |_| Ok::<(), OdpError>(()))
        .unwrap_err();
    assert_eq!(
        err,
        OdpError::UnexpectedMessageId { expected: 0x11, got: 0x99 }
    );
}
