use odp_client::OdpError;

#[test]
fn odp_error_is_copy_eq_and_debug() {
    let e = OdpError::Transport;
    // Copy + Clone
    let _ = e;
    let _ = e;
    // Eq
    assert_eq!(OdpError::Transport, OdpError::Transport);
    assert_ne!(OdpError::Transport, OdpError::Timeout);
    // Debug (just confirm it doesn't panic)
    let _ = format!("{e:?}");
}

#[test]
fn odp_error_unexpected_message_id_carries_expected_and_got() {
    let e = OdpError::UnexpectedMessageId {
        expected: 0x1234,
        got: 0x5678,
    };
    match e {
        OdpError::UnexpectedMessageId { expected, got } => {
            assert_eq!(expected, 0x1234);
            assert_eq!(got, 0x5678);
        }
        _ => panic!("wrong variant"),
    }
}
