//! ODP (Open Device Partnership) MCTP message type.
//!
//! Wire format defined by the ODP project. Message type byte `0x7D`.
//! Header layout: 32-bit bitfield, big-endian on the wire.

#![cfg(feature = "odp")]

use bit_register::bit_register;
use crate::{MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, error::MctpPacketResult};

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

bit_register! {
    #[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
    pub struct OdpHeaderWireFormat: little_endian u32 {
        pub message_id:       u16 => [0:14],
        pub is_error:          u8 => [15],
        pub service_id:        u8 => [16:23],
        pub _reserved_b24:     u8 => [24],
        pub is_request:        u8 => [25],
        pub _reserved_b26_31:  u8 => [26:31],
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OdpHeader {
    pub is_request: bool,
    pub service: OdpService,
    pub is_error: bool,
    /// Caller-supplied 15-bit message identifier. Values above `0x7FFF`
    /// are silently masked on serialize (matching legacy behavior in
    /// `odp-platform-common::ec-test-lib::serial.rs:106`).
    pub message_id: u16,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct UnknownService(pub u8);

impl OdpHeader {
    pub const HEADER_LEN: usize = 4;

    pub fn to_be_bytes(self) -> [u8; 4] {
        let w = OdpHeaderWireFormat {
            is_request: self.is_request as u8,
            service_id: self.service as u8,
            is_error: self.is_error as u8,
            message_id: self.message_id & 0x7FFF,
            ..Default::default()
        };
        let raw: u32 = w.try_into().expect("reserved fields are zero; cannot overflow");
        raw.to_be_bytes()
    }

    pub fn from_be_bytes(bytes: [u8; 4]) -> Result<Self, UnknownService> {
        let raw = u32::from_be_bytes(bytes);
        let w: OdpHeaderWireFormat = raw
            .try_into()
            .expect("primitive fields u8/u16 always parse from any u32");
        let raw_service = w.service_id;
        let service = OdpService::from_u8(raw_service).ok_or(UnknownService(raw_service))?;
        Ok(Self {
            is_request: w.is_request != 0,
            service,
            is_error: w.is_error != 0,
            message_id: w.message_id,
        })
    }
}

impl MctpMessageHeaderTrait for OdpHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        if buffer.len() < Self::HEADER_LEN {
            return Err(MctpPacketError::SerializeError(
                "buffer too small for ODP header",
            ));
        }
        buffer[..Self::HEADER_LEN].copy_from_slice(&self.to_be_bytes());
        Ok(Self::HEADER_LEN)
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        if buffer.len() < Self::HEADER_LEN {
            return Err(MctpPacketError::HeaderParseError(
                "buffer too small for ODP header",
            ));
        }
        let mut b = [0u8; 4];
        b.copy_from_slice(&buffer[..Self::HEADER_LEN]);
        let header = Self::from_be_bytes(b)
            .map_err(|_| MctpPacketError::HeaderParseError("unknown ODP service id"))?;
        Ok((header, &buffer[Self::HEADER_LEN..]))
    }
}

/// An ODP message: a typed header followed by an opaque body payload.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OdpMessage<'a> {
    pub header: OdpHeader,
    pub body: &'a [u8],
}

impl<'a> OdpMessage<'a> {
    /// Serialize the full ODP wire packet (header + body) into `out`.
    ///
    /// Returns the total number of bytes written (`OdpHeader::HEADER_LEN +
    /// self.body.len()`) on success, or `MctpPacketError::SerializeError` if
    /// the output buffer is too small.
    ///
    /// This is the convenience entry point for callers that hold a complete
    /// `OdpMessage` and want a single call to produce on-wire bytes. The
    /// framework-level trait method, `<OdpMessage as MctpMessageTrait>::serialize`,
    /// is body-only — the header is serialized separately by the packet framing
    /// layer when using the MCTP trait infrastructure. See `MctpMessageTrait`
    /// in `mctp-rs/src/message_type/mod.rs` for the framework pattern.
    pub fn serialize_with_header<M: MctpMedium>(&self, out: &mut [u8]) -> MctpPacketResult<usize, M> {
        let needed = OdpHeader::HEADER_LEN + self.body.len();
        if out.len() < needed {
            return Err(MctpPacketError::SerializeError(
                "buffer too small for ODP message",
            ));
        }
        out[..OdpHeader::HEADER_LEN].copy_from_slice(&self.header.to_be_bytes());
        out[OdpHeader::HEADER_LEN..needed].copy_from_slice(self.body);
        Ok(needed)
    }
}

/// Body-only serialization per the `MctpMessageTrait` framework contract.
/// The header is serialized separately at the packet framing layer.
/// Use [`OdpMessage::serialize_with_header`] for a single-call full-packet
/// serialization.
impl<'buf> MctpMessageTrait<'buf> for OdpMessage<'buf> {
    type Header = OdpHeader;
    const MESSAGE_TYPE: u8 = ODP_MESSAGE_TYPE;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        if buffer.len() < self.body.len() {
            return Err(MctpPacketError::SerializeError(
                "buffer too small for ODP message body",
            ));
        }
        buffer[..self.body.len()].copy_from_slice(self.body);
        Ok(self.body.len())
    }

    fn deserialize<M: MctpMedium>(
        header: &Self::Header,
        buffer: &'buf [u8],
    ) -> MctpPacketResult<Self, M> {
        Ok(OdpMessage {
            header: *header,
            body: buffer,
        })
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

    #[test]
    fn odp_header_round_trip_via_wire_bytes() {
        let original = OdpHeader {
            is_request: true,
            service: OdpService::Battery,
            is_error: false,
            message_id: 0x1234,
        };
        let bytes = original.to_be_bytes();
        let parsed = OdpHeader::from_be_bytes(bytes).expect("known service");
        assert_eq!(parsed, original);
    }

    #[test]
    fn odp_header_bit_layout_is_stable() {
        // Snapshot matches odp-platform-common::ec-test-lib::serial.rs:509:
        //   build_odp_header(is_request=true, service=Battery=0x08, msg_id=2)
        //     => [0x02, 0x08, 0x00, 0x02]
        let h = OdpHeader {
            is_request: true,
            service: OdpService::Battery,
            is_error: false,
            message_id: 2,
        };
        assert_eq!(h.to_be_bytes(), [0x02, 0x08, 0x00, 0x02]);
    }

    #[test]
    fn odp_header_response_round_trip() {
        // is_request=false (response), is_error=false, msg_id=2 — mirrors serial.rs:530.
        let h = OdpHeader {
            is_request: false,
            service: OdpService::Battery,
            is_error: false,
            message_id: 2,
        };
        let parsed = OdpHeader::from_be_bytes(h.to_be_bytes()).expect("known service");
        assert_eq!(parsed, h);
    }

    #[test]
    fn odp_header_rejects_unknown_service() {
        // service_id 0xFF is not in the known set.
        let raw_u32: u32 = (1 << 25) | (0xFF << 16) | 1;
        let bytes = raw_u32.to_be_bytes();
        assert!(matches!(
            OdpHeader::from_be_bytes(bytes),
            Err(UnknownService(0xFF))
        ));
    }

    #[test]
    fn odp_message_serializes_header_then_body() {
        use crate::test_util::TestMedium;
        let msg = OdpMessage {
            header: OdpHeader {
                service: OdpService::Battery,
                is_error: false,
                message_id: 7,
                is_request: true,
            },
            body: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut out = [0u8; 32];
        let n = msg.serialize_with_header::<TestMedium>(&mut out).unwrap();
        assert_eq!(n, OdpHeader::HEADER_LEN + 4);
        assert_eq!(&out[..OdpHeader::HEADER_LEN], &msg.header.to_be_bytes());
        assert_eq!(&out[OdpHeader::HEADER_LEN..OdpHeader::HEADER_LEN + 4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn odp_header_implements_mctp_message_header_trait() {
        use crate::test_util::TestMedium;
        use crate::MctpMessageHeaderTrait;

        let h = OdpHeader {
            is_request: true,
            service: OdpService::Battery,
            is_error: false,
            message_id: 0x42,
        };
        let mut buf = [0u8; 4];
        let written = h.serialize::<TestMedium>(&mut buf).unwrap();
        assert_eq!(written, 4);
        let (parsed, rest) = OdpHeader::deserialize::<TestMedium>(&buf).unwrap();
        assert_eq!(rest.len(), 0);
        assert_eq!(parsed, h);
    }

    #[test]
    fn wire_snapshot_matches_legacy_impls_battery_get_bst() {
        // Source of truth: odp-platform-common/ec/test-lib/src/serial.rs:505-510
        //   build_odp_header(is_request=true, service=Battery=0x08, msg_id=2)
        //   produces [0x02, 0x08, 0x00, 0x02].
        let h = OdpHeader {
            is_request: true,
            service: OdpService::Battery,
            is_error: false,
            message_id: 2,
        };
        assert_eq!(h.to_be_bytes(), [0x02, 0x08, 0x00, 0x02]);
    }

    #[test]
    fn wire_snapshot_matches_legacy_impls_thermal_request() {
        // Source: serial.rs:513-516.
        let h = OdpHeader {
            is_request: true,
            service: OdpService::Thermal,
            is_error: false,
            message_id: 1,
        };
        assert_eq!(h.to_be_bytes(), [0x02, 0x09, 0x00, 0x01]);
    }

    #[test]
    fn wire_snapshot_matches_legacy_impls_time_alarm_15bit_id() {
        // Source: serial.rs:520-526. service_id=0x0B, msg_id=0x1234.
        let h = OdpHeader {
            is_request: true,
            service: OdpService::TimeAlarm,
            is_error: false,
            message_id: 0x1234,
        };
        let bytes = h.to_be_bytes();
        let parsed = OdpHeader::from_be_bytes(bytes).expect("known service");
        assert_eq!(parsed, h);
        // raw_u32 = (1<<25) | (0x0B<<16) | 0x1234 = 0x020B1234 → [0x02, 0x0B, 0x12, 0x34]
        assert_eq!(bytes, [0x02, 0x0B, 0x12, 0x34]);
    }

    #[test]
    fn wire_snapshot_battery_response() {
        // Source: serial.rs:530-538. is_request=false, service=Battery, msg_id=2.
        let h = OdpHeader {
            is_request: false,
            service: OdpService::Battery,
            is_error: false,
            message_id: 2,
        };
        // raw_u32 = (0<<25) | (0x08<<16) | 2 = 0x00080002 → [0x00, 0x08, 0x00, 0x02]
        assert_eq!(h.to_be_bytes(), [0x00, 0x08, 0x00, 0x02]);
    }
}
