//! ODP (Open Device Partnership) MCTP message type.
//!
//! Wire format defined by the ODP project. Message type byte `0x7D`.
//! Header layout: 32-bit bitfield, little-endian on the wire.

#![cfg(feature = "odp")]

/// MCTP message type byte assigned to ODP traffic.
pub const ODP_MESSAGE_TYPE: u8 = 0x7D;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OdpService {
    Battery   = 0x08,
    Thermal   = 0x09,
    TimeAlarm = 0x0B,
}

impl OdpService {
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0x08 => Some(Self::Battery),
            0x09 => Some(Self::Thermal),
            0x0B => Some(Self::TimeAlarm),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_constant_is_0x7d() {
        assert_eq!(ODP_MESSAGE_TYPE, 0x7D);
    }

    #[test]
    fn odp_service_known_codes_round_trip() {
        for (raw, service) in [
            (0x08, OdpService::Battery),
            (0x09, OdpService::Thermal),
            (0x0B, OdpService::TimeAlarm),
        ] {
            assert_eq!(OdpService::from_u8(raw), Some(service));
            assert_eq!(service as u8, raw);
        }
    }

    #[test]
    fn odp_service_unknown_code_is_none() {
        assert_eq!(OdpService::from_u8(0xFF), None);
    }
}
