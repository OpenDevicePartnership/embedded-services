use odp_client::{LoopbackTransport, OdpError, OdpTransport};

#[test]
fn loopback_echoes_what_you_send() {
    let mut t = LoopbackTransport::new();
    t.send_message(&[9, 8, 7]).unwrap();
    let mut buf = [0u8; 8];
    let n = t.recv_message(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[9, 8, 7]);
}

#[test]
fn loopback_recv_with_no_pending_message_returns_transport_error() {
    let mut t = LoopbackTransport::new();
    let mut buf = [0u8; 8];
    let err = t.recv_message(&mut buf).unwrap_err();
    assert_eq!(err, OdpError::Transport);
}

#[test]
fn loopback_recv_with_too_small_buffer_returns_buffer_too_small() {
    let mut t = LoopbackTransport::new();
    t.send_message(&[1, 2, 3, 4, 5]).unwrap();
    let mut buf = [0u8; 2];
    let err = t.recv_message(&mut buf).unwrap_err();
    assert_eq!(err, OdpError::BufferTooSmall);
}

#[test]
fn loopback_send_overflow_returns_buffer_too_small() {
    let mut t = LoopbackTransport::new();
    let huge = [0u8; 512]; // larger than the 256-byte internal buffer
    let err = t.send_message(&huge).unwrap_err();
    assert_eq!(err, OdpError::BufferTooSmall);
}

#[test]
fn loopback_send_then_recv_is_idempotent_across_messages() {
    let mut t = LoopbackTransport::new();
    // Round 1
    t.send_message(&[1, 2]).unwrap();
    let mut buf = [0u8; 8];
    let n = t.recv_message(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[1, 2]);
    // Round 2 — should work after the previous recv drained the buffer
    t.send_message(&[3, 4, 5]).unwrap();
    let n = t.recv_message(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[3, 4, 5]);
}
