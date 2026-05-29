use odp_client::{OdpError, OdpTransport};

struct DummyTransport(Option<Vec<u8>>);

impl OdpTransport for DummyTransport {
    fn send_message(&mut self, payload: &[u8]) -> Result<(), OdpError> {
        self.0 = Some(payload.to_vec());
        Ok(())
    }
    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError> {
        let v = self.0.take().ok_or(OdpError::Transport)?;
        if buf.len() < v.len() {
            return Err(OdpError::BufferTooSmall);
        }
        buf[..v.len()].copy_from_slice(&v);
        Ok(v.len())
    }
}

#[test]
fn dummy_transport_satisfies_the_trait() {
    let mut t = DummyTransport(None);
    t.send_message(&[1, 2, 3]).unwrap();
    let mut buf = [0u8; 8];
    let n = t.recv_message(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[1, 2, 3]);
}

#[test]
fn dummy_transport_recv_with_no_pending_message_returns_transport_error() {
    let mut t = DummyTransport(None);
    let mut buf = [0u8; 8];
    let err = t.recv_message(&mut buf).unwrap_err();
    assert_eq!(err, OdpError::Transport);
}

#[test]
fn dummy_transport_recv_with_too_small_buffer_returns_buffer_too_small() {
    let mut t = DummyTransport(None);
    t.send_message(&[1, 2, 3, 4, 5]).unwrap();
    let mut buf = [0u8; 2];
    let err = t.recv_message(&mut buf).unwrap_err();
    assert_eq!(err, OdpError::BufferTooSmall);
}
