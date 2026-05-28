//! ODP (Open Device Partnership) MCTP message type.
//!
//! Wire format defined by the ODP project. Message type byte `0x7D`.
//! Header layout: 32-bit bitfield, little-endian on the wire.

#![cfg(feature = "odp")]

/// MCTP message type byte assigned to ODP traffic.
pub const ODP_MESSAGE_TYPE: u8 = 0x7D;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_constant_is_0x7d() {
        assert_eq!(ODP_MESSAGE_TYPE, 0x7D);
    }
}
