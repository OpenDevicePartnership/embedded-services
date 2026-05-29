use mctp_rs::{EC_EID, SP_EID};
use odp_client::{OdpTransport, SerialTransport};

struct LoopbackUart {
    buf: Vec<u8>,
    cursor: usize,
}

impl LoopbackUart {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
        }
    }
}

impl embedded_io::ErrorType for LoopbackUart {
    type Error = core::convert::Infallible;
}

impl embedded_io::Read for LoopbackUart {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let avail = self.buf.len() - self.cursor;
        if avail == 0 {
            return Ok(0);
        }
        let n = avail.min(out.len());
        out[..n].copy_from_slice(&self.buf[self.cursor..self.cursor + n]);
        self.cursor += n;
        Ok(n)
    }
}

impl embedded_io::Write for LoopbackUart {
    fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn serial_transport_round_trips_one_message() {
    // Hand in ODP wire bytes (4-byte header + body); the transport
    // frames them before writing to the UART and strips the framing
    // on read, returning the original ODP bytes.
    let uart = LoopbackUart::new();
    let mut t = SerialTransport::new(uart, SP_EID, EC_EID);

    // Arbitrary 6 bytes: first 4 are the OdpHeader byte pattern, last 2
    // are the body. The transport does not semantically validate the
    // header — it just blits bytes — so a service-id (0x55) that is not a
    // real OdpService variant works here.
    let payload = [0x01u8, 0x55, 0x00, 0x00, 0xDE, 0xAD];
    t.send_message(&payload).unwrap();

    let mut out = [0u8; 64];
    let n = t.recv_message(&mut out).unwrap();
    assert_eq!(&out[..n], &payload);
}

#[test]
fn serial_transport_send_with_too_short_payload_errors() {
    // Header is 4 bytes; a payload shorter than that has no valid
    // header bytes to wrap and must be rejected.
    let uart = LoopbackUart::new();
    let mut t = SerialTransport::new(uart, SP_EID, EC_EID);
    let err = t.send_message(&[0x01, 0x02]).unwrap_err();
    assert_eq!(err, odp_client::OdpError::BufferTooSmall);
}
