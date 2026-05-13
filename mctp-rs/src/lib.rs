#![no_std]
#![allow(dead_code)]
// extern crate std;
//! # mctp-rs
//!
//! A `no_std` Rust implementation of the [DMTF Management Component Transport Protocol (MCTP)](https://www.dmtf.org/sites/default/files/standards/documents/DSP0236_1.3.3.pdf)
//! transport.
//!
//! ## Receiving and parsing messages
//!
//! Use `MctpPacketContext` with your medium to assemble messages from packets.
//!
//! ```rust,no_run
//! # use mctp_rs::*;
//! # #[derive(Debug, Clone, Copy)] struct MyMedium { mtu: usize }
//! # #[derive(Debug, Clone, Copy)] struct MyMediumFrame { packet_size: usize }
//! # impl MctpMedium for MyMedium { type Frame=MyMediumFrame; type Error=&'static str; type ReplyContext=(); type Encoding=PassthroughEncoding;
//! #   fn max_message_body_size(&self)->usize{self.mtu}
//! #   fn deserialize<'b>(&self,p:&'b [u8])->MctpPacketResult<(Self::Frame,EncodingDecoder<'b,Self::Encoding>),Self>{Ok((MyMediumFrame{packet_size:p.len()},EncodingDecoder::new(p)))}
//! #   fn serialize<'b,F>(&self,_:Self::ReplyContext,b:&'b mut [u8],w:F)->MctpPacketResult<&'b [u8],Self> where F: for<'a> FnOnce(&mut EncodingEncoder<'a,Self::Encoding>)->MctpPacketResult<(),Self>{let n={let mut e=EncodingEncoder::<Self::Encoding>::new(b);w(&mut e)?;e.wire_position()};Ok(&b[..n])}}
//! # impl MctpMediumFrame<MyMedium> for MyMediumFrame { fn packet_size(&self)->usize{self.packet_size} fn reply_context(&self)->(){()}}
//! let mut assembly_buffer = [0u8; 1024];
//! let medium = MyMedium { mtu: 256 };
//! let mut context = MctpPacketContext::new(medium, &mut assembly_buffer);
//!
//! // Typically obtained from your bus
//! let raw_packet_data: &[u8] = &[0x01, 0x02, 0x03, 0x83];
//!
//! match context.deserialize_packet(raw_packet_data) {
//!     Ok(Some(message)) => {
//!         // We received a complete MCTP message
//!         if let Ok((header, control)) = message.parse_as::<MctpControl>() {
//!             match control {
//!                 MctpControl::GetEndpointIdRequest => {
//!                     // Handle control request
//!                     let _instance = header.instance_id;
//!                 }
//!                 MctpControl::GetEndpointIdResponse(bytes3) => {
//!                     // Use response payload (3 bytes as per spec)
//!                     let _eid = bytes3[0];
//!                 }
//!                 _ => {}
//!             }
//!         }
//!     }
//!     Ok(None) => { /* partial message; wait for more packets */ }
//!     Err(e) => {
//!         // handle protocol/medium error
//!         let _ = e;
//!     }
//! }
//! ```
//! ## Sending messages
//!
//! Construct a header + body pair implementing `MctpMessageTrait` and serialize to one or
//! more packets using `serialize_packet`.
//!
//! ```rust,no_run
//! # use mctp_rs::*;
//! # #[derive(Debug, Clone, Copy)] struct MyMedium { mtu: usize }
//! # #[derive(Debug, Clone, Copy)] struct MyMediumFrame { packet_size: usize }
//! # impl MctpMedium for MyMedium { type Frame=MyMediumFrame; type Error=&'static str; type ReplyContext=(); type Encoding=PassthroughEncoding; fn max_message_body_size(&self)->usize{self.mtu}
//! #   fn deserialize<'b>(&self,p:&'b [u8])->MctpPacketResult<(Self::Frame,EncodingDecoder<'b,Self::Encoding>),Self>{Ok((MyMediumFrame{packet_size:p.len()},EncodingDecoder::new(p)))}
//! #   fn serialize<'b,F>(&self,_:Self::ReplyContext,b:&'b mut [u8],w:F)->MctpPacketResult<&'b [u8],Self> where F: for<'a> FnOnce(&mut EncodingEncoder<'a,Self::Encoding>)->MctpPacketResult<(),Self>{let n={let mut e=EncodingEncoder::<Self::Encoding>::new(b);w(&mut e)?;e.wire_position()};Ok(&b[..n])}}
//! # impl MctpMediumFrame<MyMedium> for MyMediumFrame { fn packet_size(&self)->usize{self.packet_size} fn reply_context(&self)->(){()}}
//! let mut buf = [0u8; 1024];
//! let mut ctx = MctpPacketContext::new(MyMedium { mtu: 64 }, &mut buf);
//!
//! let reply = MctpReplyContext {
//!     destination_endpoint_id: EndpointId::try_from(0x20).unwrap(),
//!     source_endpoint_id: EndpointId::try_from(0x21).unwrap(),
//!     packet_sequence_number: MctpSequenceNumber::new(0),
//!     message_tag: MctpMessageTag::try_from(1).unwrap(),
//!     medium_context: (),
//! };
//!
//! let message = (
//!     VendorDefinedPciHeader(0x1234),
//!     VendorDefinedPci(&[0xDE, 0xAD, 0xBE, 0xEF]),
//! );
//!
//! let mut packets = ctx.serialize_packet(reply, message).unwrap();
//! while let Some(packet_result) = packets.next() {
//!     let packet_bytes = packet_result.unwrap();
//!     // send `packet_bytes` via your bus
//!     let _ = packet_bytes;
//! }
//! ```
//!
//! ## Implementing a custom medium
//!
//! The crate is transport-agnostic via the `MctpMedium` trait. Implement it for your bus
//! (e.g., SMBus, eSPI) and provide a frame type implementing `MctpMediumFrame`.
//!
//! ```rust,no_run
//! use mctp_rs::*;
//!
//! #[derive(Debug, Clone, Copy)]
//! struct MyMedium {
//!     mtu: usize,
//! }
//!
//! #[derive(Debug, Clone, Copy)]
//! struct MyMediumFrame {
//!     packet_size: usize,
//! }
//!
//! impl MctpMedium for MyMedium {
//!     type Frame = MyMediumFrame;
//!     type Error = &'static str;
//!     type ReplyContext = ();
//!     type Encoding = PassthroughEncoding;
//!
//!     fn max_message_body_size(&self) -> usize {
//!         self.mtu
//!     }
//!
//!     fn deserialize<'buf>(
//!         &self,
//!         packet: &'buf [u8],
//!     ) -> MctpPacketResult<(Self::Frame, EncodingDecoder<'buf, Self::Encoding>), Self> {
//!         // Strip/validate transport headers as needed for your bus and return MCTP payload slice
//!         Ok((
//!             MyMediumFrame {
//!                 packet_size: packet.len(),
//!             },
//!             EncodingDecoder::new(packet),
//!         ))
//!     }
//!
//!     fn serialize<'buf, F>(
//!         &self,
//!         _reply_context: Self::ReplyContext,
//!         buffer: &'buf mut [u8],
//!         message_writer: F,
//!     ) -> MctpPacketResult<&'buf [u8], Self>
//!     where
//!         F: for<'a> FnOnce(
//!             &mut EncodingEncoder<'a, Self::Encoding>,
//!         ) -> MctpPacketResult<(), Self>,
//!     {
//!         // Prepend transport headers as needed, then ask the writer to write MCTP payload
//!         let written = {
//!             let mut encoder = EncodingEncoder::<Self::Encoding>::new(buffer);
//!             message_writer(&mut encoder)?;
//!             encoder.wire_position()
//!         };
//!         Ok(&buffer[..written])
//!     }
//! }
//!
//! impl MctpMediumFrame<MyMedium> for MyMediumFrame {
//!     fn packet_size(&self) -> usize {
//!         self.packet_size
//!     }
//!     fn reply_context(&self) -> <MyMedium as MctpMedium>::ReplyContext {
//!         ()
//!     }
//! }
//! ```

mod buffer_encoding;
mod deserialize;
mod endpoint_id;
pub mod error;
mod mctp_command_code;
pub mod mctp_completion_code;
mod mctp_message_tag;
mod mctp_packet_context;
mod mctp_sequence_number;
mod mctp_transport_header;
mod medium;
mod message_type;
mod serialize;
#[cfg(test)]
mod test_util;

pub use buffer_encoding::{
    BufferEncoding, DecodeError, EncodeError, EncodingDecoder, EncodingEncoder, PassthroughEncoding,
};
pub use endpoint_id::EndpointId;
pub use error::{MctpPacketError, MctpPacketResult};
pub use mctp_message_tag::MctpMessageTag;
pub use mctp_packet_context::{MctpPacketContext, MctpReplyContext};
pub use mctp_sequence_number::MctpSequenceNumber;
#[cfg(feature = "serial")]
pub use medium::serial::{CONST_MTU, EC_EID, MctpSerialMedium, MctpSerialMediumFrame, SP_EID, SerialEncoding};
pub use medium::*;
pub use message_type::*;

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpMessage<'buffer, M: MctpMedium> {
    pub reply_context: MctpReplyContext<M>,
    pub message_buffer: MctpMessageBuffer<'buffer>,
    pub message_integrity_check: Option<u8>,
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpMessageBuffer<'buffer> {
    integrity_check: u8,
    message_type: u8,
    rest: &'buffer [u8],
}

impl<'buffer, M: MctpMedium> MctpMessage<'buffer, M> {
    pub fn can_parse_as<P: MctpMessageTrait<'buffer>>(&self) -> bool {
        self.message_buffer.message_type == P::MESSAGE_TYPE
    }
    pub fn parse_as<P: MctpMessageTrait<'buffer>>(&self) -> MctpPacketResult<(P::Header, P), M> {
        if !self.can_parse_as::<P>() {
            return Err(MctpPacketError::HeaderParseError("message type mismatch"));
        }
        let (header, rest) = P::Header::deserialize(self.message_buffer.rest)?;
        let message = P::deserialize(&header, rest)?;
        Ok((header, message))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{
        error::ProtocolError, mctp_command_code::MctpControlCommandCode, mctp_packet_context::MctpPacketContext,
        test_util::*,
    };

    struct Packet(&'static [u8]);
    const GET_ENDPOINT_ID_PACKET_NO_EOM: Packet = Packet(&[
        // test medium frame (header + trailer): 0 bytes
        // transport header:
        0b0000_0001, // mctp reserved, header version
        0b0000_1001, // destination endpoint id (9)
        0b0001_0110, // source endpoint id (22)
        0b1000_0011, // som, eom, seq (0), to, tag (3)
        // message header:
        0b0000_0000, // integrity check (off) / message type (MessageType::MctpControl)
        0b0000_0000, // rq, d, rsvd, instance id
        0b0000_0010, // command code (2: get endpoint id)
        0b0000_0000, // completion code
        // message body:
        0b0000_1111, // endpoint id (15)
        0b0000_0001, /* endpoint type (simple = 0b00) / endpoint id type (static eid supported =
                      * 0b01) */
        0b1111_0000, // medium specific
    ]);

    const EMPTY_PACKET_EOM: Packet = Packet(&[
        // transport header:
        0b0000_0001, // mctp reserved, header version
        0b0000_1001, // destination endpoint id (9)
        0b0001_0110, // source endpoint id (14)
        0b0101_0011, // som, eom, seq (1), to, tag (3)
    ]);

    #[test]
    fn split_over_two_packets() {
        let mut buffer = [0; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        assert_eq!(
            context.deserialize_packet(GET_ENDPOINT_ID_PACKET_NO_EOM.0).unwrap(),
            None
        );

        let message = context.deserialize_packet(EMPTY_PACKET_EOM.0).unwrap().unwrap();

        assert_eq!(message.can_parse_as::<MctpControl>(), true);
        assert_eq!(message.message_integrity_check, None);
        assert_eq!(
            message.reply_context,
            MctpReplyContext {
                destination_endpoint_id: EndpointId::Id(9),
                source_endpoint_id: EndpointId::Id(22),
                packet_sequence_number: MctpSequenceNumber::new(1),
                message_tag: MctpMessageTag::try_from(3).unwrap(),
                medium_context: (),
            }
        );
        assert_eq!(
            message.parse_as().unwrap(),
            (
                MctpControlHeader {
                    command_code: MctpControlCommandCode::GetEndpointId,
                    ..Default::default()
                },
                MctpControl::GetEndpointIdResponse([
                    0b0000_1111, // endpoint id (15)
                    0b0000_0001, /* endpoint type (simple = 0b00) / endpoint id type (static eid
                                  * supported = 0b01) */
                    0b1111_0000, // medium specific
                ]),
            )
        );
    }

    #[test]
    fn lacking_start_of_message() {
        let mut buffer = [0; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        assert_eq!(
            context.deserialize_packet(&[
                // transport header:
                0b0000_0000, // mctp reserved, header version
                0b0000_0000, // destination endpoint id
                0b0000_0000, // source endpoint id
                0b0000_0000, // som, eom, seq (0), to, tag
            ]),
            Err(MctpPacketError::ProtocolError(ProtocolError::ExpectedStartOfMessage,))
        );
    }

    #[test]
    fn repeated_start_of_message() {
        let mut buffer = [0; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        context.deserialize_packet(GET_ENDPOINT_ID_PACKET_NO_EOM.0).unwrap();

        assert_eq!(
            context.deserialize_packet(&[
                // transport header:
                0b0000_0000, // mctp reserved, header version
                0b0000_0000, // destination endpoint id
                0b0000_0000, // source endpoint id
                0b1000_0000, // som, eom, seq (0), to, tag
            ]),
            Err(MctpPacketError::ProtocolError(ProtocolError::UnexpectedStartOfMessage,))
        );
    }

    #[test]
    fn message_tag_mismatch() {
        let mut buffer = [0; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        // message tag = 0
        context.deserialize_packet(GET_ENDPOINT_ID_PACKET_NO_EOM.0).unwrap();

        // message tag = 1
        assert_eq!(
            context.deserialize_packet(&[
                // transport header:
                0b0000_0000, // mctp reserved, header version
                0b0000_0000, // destination endpoint id
                0b0000_0000, // source endpoint id
                0b0101_0010, // som, eom, seq (1), to, tag (2)
            ]),
            Err(MctpPacketError::ProtocolError(ProtocolError::MessageTagMismatch(
                MctpMessageTag::try_from(3).unwrap(),
                MctpMessageTag::try_from(2).unwrap(),
            ),))
        );
    }

    #[test]
    fn test_send_packet() {
        let mut buffer = [0; 1024];
        let mut context =
            MctpPacketContext::<TestMedium>::new(TestMedium::new().with_headers(&[0xA, 0xB], &[0xC, 0xD]), &mut buffer);

        let reply_context = MctpReplyContext {
            destination_endpoint_id: EndpointId::try_from(236).unwrap(),
            source_endpoint_id: EndpointId::try_from(192).unwrap(),
            packet_sequence_number: MctpSequenceNumber::new(1),
            message_tag: MctpMessageTag::try_from(3).unwrap(),
            medium_context: (),
        };

        let message = (VendorDefinedPciHeader(0x1234), VendorDefinedPci(&[0xA5, 0xB6]));

        let mut state = context.serialize_packet(reply_context, message).unwrap();

        let packet = state.next().unwrap().unwrap();
        assert_eq!(
            &[
                // test header - 2 bytes
                0xA,
                0xB,
                // mctp transport header
                0b0000_0001, // mctp reserved, header version
                192,         // destination endpoint id
                236,         // source endpoint id
                0b1110_0011, // som (1), eom (1), seq (2), tag owner (0), message tag (3)
                // mctp message header - 3 bytes
                0x7E, // integrity check (0), message type (vendor defined pci)
                0x12, // pci vendor id - low byte
                0x34, // pci vendor id - high byte
                // mctp message body - 1 byte
                0xA5,
                0xB6,
                // test trailer - 2 bytes
                0xC,
                0xD,
            ],
            packet
        );
    }

    #[test]
    fn test_send_packet_multi() {
        const MTU_SIZE: usize = 14;
        let mut buffer = [0; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(
            TestMedium::new()
                .with_headers(&[0xA, 0xB], &[0xC, 0xD])
                // 4 bytes transport header + 4 bytes of data
                .with_mtu(MTU_SIZE),
            &mut buffer,
        );

        let reply_context = MctpReplyContext {
            destination_endpoint_id: EndpointId::try_from(236).unwrap(),
            source_endpoint_id: EndpointId::try_from(192).unwrap(),
            packet_sequence_number: MctpSequenceNumber::new(1),
            message_tag: MctpMessageTag::try_from(3).unwrap(),
            medium_context: (),
        };

        // 10 byte to send over 3 packets
        let data_to_send = [0xA5, 0xB6, 0xC7, 0xD8, 0xE9, 0xFA, 0x0B, 0x1C, 0x2D, 0x3E];
        let message = (VendorDefinedPciHeader(0x1234), VendorDefinedPci(&data_to_send));

        let mut state = context.serialize_packet(reply_context, message).unwrap();

        // First packet
        let packet1 = state.next().unwrap().unwrap();
        let expected: [u8; MTU_SIZE] = [
            // test header - 2 bytes
            0xA,
            0xB,
            // mctp transport header - 4 bytes
            0b0000_0001, // mctp reserved, header version
            192,         // destination endpoint id
            236,         // source endpoint id
            0b1010_0011, // som (1), eom (0), seq (2), tag owner (0), message tag (3)
            // mctp message header - 3 bytes
            0x7E, // integrity check (0), message type (vendor defined pci)
            0x12, // pci vendor id - low byte
            0x34, // pci vendor id - high byte
            // mctp message body data - 1 bytes
            0xA5,
            0xB6,
            0xC7,
            // test trailer - 2 bytes
            0xC,
            0xD,
        ];
        assert_eq!(packet1, &expected[..MTU_SIZE]);

        // Second packet (middle packet with 4 bytes of data)
        let packet2 = state.next().unwrap().unwrap();
        let expected: [u8; MTU_SIZE] = [
            // test header - 2 bytes
            0xA,
            0xB,
            // mctp transport header - 4 bytes
            0b0000_0001, // mctp reserved, header version
            192,         // destination endpoint id
            236,         // source endpoint id
            0b0011_0011, // som (0), eom (0), seq (3), tag owner (0), message tag (3)
            // mctp body data - 4 bytes
            0xD8,
            0xE9,
            0xFA,
            0x0B,
            0x1C,
            0x2D,
            // test trailer - 2 bytes
            0xC,
            0xD,
        ];
        assert_eq!(packet2, &expected[..]);

        // Third packet (final packet with 2 bytes of data)
        let packet3 = state.next().unwrap().unwrap();
        let expected: [u8; MTU_SIZE] = [
            // test header - 2 bytes
            0xA,
            0xB,
            // mctp transport header - 4 bytes
            0b0000_0001, // mctp reserved, header version
            192,         // destination endpoint id
            236,         // source endpoint id
            0b0100_0011, // som (0), eom (1), seq (0), tag owner (0), message tag (3)
            // mctp body data - 1 bytes
            0x3E,
            // test trailer - 2 bytes
            0xC,
            0xD,
            // remainder is not populated
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        assert_eq!(packet3, &expected[..9]);

        // Verify no more packets
        let next = state.next();
        assert!(next.is_none(), "Expected exactly 3 packets: {next:x?}");
    }

    #[test]
    fn test_buffer_overflow_protection() {
        // Test that buffer overflow is properly prevented
        let mut small_buffer = [0u8; 16]; // Very small buffer
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut small_buffer);

        // Create a packet that would cause overflow without protection
        let large_packet = [
            // transport header:
            0b0000_0001, // mctp reserved, header version
            0b0000_1001, // destination endpoint id (9)
            0b0001_0110, // source endpoint id (22)
            0b1000_0011, // som, eom, seq (0), to, tag (3)
            // message header:
            0b0000_0000, // integrity check (off) / message type (mctp control message)
            0b0000_0000, // rq, d, rsvd, instance id
            0b0000_0010, // command code (2: get endpoint id)
            0b0000_0000, // completion code
            // Large message body that would overflow small buffer
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0A,
            0x0B,
            0x0C,
            0x0D,
            0x0E,
            0x0F,
            0x10,
        ];

        // This should return an error instead of panicking
        let result = context.deserialize_packet(&large_packet);
        assert!(result.is_err());

        if let Err(MctpPacketError::HeaderParseError(msg)) = result {
            assert!(msg.contains("buffer overflow"));
        } else {
            panic!("Expected HeaderParseError with buffer overflow message");
        }
    }

    #[test]
    fn test_multi_packet_buffer_overflow() {
        // Test buffer overflow with multiple packets
        let mut small_buffer = [0u8; 20]; // Small buffer that can fit first packet but not second
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut small_buffer);

        // First packet - fits in buffer
        let first_packet = [
            // transport header:
            0b0000_0001, // mctp reserved, header version
            0b0000_1001, // destination endpoint id (9)
            0b0001_0110, // source endpoint id (22)
            0b1000_0011, // som (1), eom (0), seq (0), to, tag (3)
            // message header:
            0b0000_0000, // integrity check (off) / message type (mctp control message)
            0b0000_0000, // rq, d, rsvd, instance id
            0b0000_0010, // command code (2: get endpoint id)
            0b0000_0000, // completion code
            // Small message body
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
        ];

        // First packet should succeed
        let result1 = context.deserialize_packet(&first_packet);
        assert!(result1.is_ok());
        assert!(result1.unwrap().is_none()); // No complete message yet

        // Second packet - would cause overflow
        let second_packet = [
            // transport header:
            0b0000_0001, // mctp reserved, header version
            0b0000_1001, // destination endpoint id (9)
            0b0001_0110, // source endpoint id (22)
            0b0101_0011, // som (0), eom (1), seq (1), to, tag (3) - correct sequence number
            // Large continuation that would overflow
            0x09,
            0x0A,
            0x0B,
            0x0C,
            0x0D,
            0x0E,
            0x0F,
            0x10,
            0x11,
            0x12,
            0x13,
            0x14,
            0x15,
            0x16,
            0x17,
            0x18,
        ];

        // Second packet should fail with buffer overflow
        let result2 = context.deserialize_packet(&second_packet);
        assert!(result2.is_err());

        if let Err(MctpPacketError::HeaderParseError(msg)) = result2 {
            assert!(msg.contains("buffer overflow"));
        } else {
            panic!("Expected HeaderParseError with buffer overflow message");
        }
    }

    #[test]
    fn test_transport_header_underflow() {
        // Test transport header parsing with insufficient bytes
        let mut buffer = [0u8; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        // Packet too short for transport header (only 3 bytes)
        let short_packet = [0x01, 0x02, 0x03];

        let result = context.deserialize_packet(&short_packet);
        assert!(result.is_err());

        if let Err(MctpPacketError::HeaderParseError(msg)) = result {
            assert!(msg.contains("cannot parse transport header"));
        } else {
            panic!("Expected HeaderParseError for short transport header");
        }
    }

    #[test]
    fn test_message_header_underflow() {
        // Test message body parsing with insufficient bytes for message header
        let mut buffer = [0u8; 1024];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut buffer);

        // Packet with transport header but no message header
        let incomplete_packet = [
            // transport header only (4 bytes)
            0b0000_0001, // mctp reserved, header version
            0b0000_1001, // destination endpoint id (9)
            0b0001_0110, // source endpoint id (22)
            0b1110_0011, /* som (1), eom (1), seq (0), to, tag (3)
                          * No message header (need 4 more bytes) */
        ];

        let result = context.deserialize_packet(&incomplete_packet);
        assert!(result.is_err());

        if let Err(MctpPacketError::HeaderParseError(msg)) = result {
            assert!(msg.contains("packet too small"), "msg: {msg}");
        } else {
            panic!("Expected HeaderParseError for short message header");
        }
    }

    #[test]
    fn test_serialize_buffer_underflow() {
        // Test serialization with buffer too small for serializing the packet and having enough
        // buffer for assembling packets
        let mut tiny_buffer = [0u8; 4]; // Too small for 4-byte transport header
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut tiny_buffer);

        let reply_context = MctpReplyContext {
            destination_endpoint_id: EndpointId::try_from(236).unwrap(),
            source_endpoint_id: EndpointId::try_from(192).unwrap(),
            packet_sequence_number: MctpSequenceNumber::new(1),
            message_tag: MctpMessageTag::try_from(3).unwrap(),
            medium_context: (),
        };

        let message = (VendorDefinedPciHeader(0x1234), VendorDefinedPci(&[0xA5]));
        let state_result = context.serialize_packet(reply_context, message);
        assert!(state_result.is_ok(), "{state_result:?}");

        let mut state = state_result.unwrap();
        let packet_result = state.next().unwrap();

        // Should fail because buffer is too small for transport header
        assert!(packet_result.is_err());
        if let Err(MctpPacketError::SerializeError(msg)) = packet_result {
            assert!(msg.contains("assembly buffer too small"));
        } else {
            panic!("Expected SerializeError for small buffer");
        }
    }

    #[test]
    fn test_zero_size_assembly_buffer() {
        // Test with zero-size assembly buffer
        let mut empty_buffer = [0u8; 0];
        let mut context = MctpPacketContext::<TestMedium>::new(TestMedium::new(), &mut empty_buffer);

        let packet = [
            0b0000_0001, // mctp reserved, header version
            0b0000_1001, // destination endpoint id (9)
            0b0001_0110, // source endpoint id (22)
            0b1110_0011, // som (1), eom (1), seq (0), to, tag (3)
            0x7F,        // message header - 3 bytes (vendor defined pci)
            0x12,        // pci vendor id - low byte
            0x34,        // pci vendor id - high byte
            0b0000_0010,
            0b0000_0000,
        ];

        let result = context.deserialize_packet(&packet);
        assert!(result.is_err());

        if let Err(MctpPacketError::HeaderParseError(msg)) = result {
            assert!(msg.contains("buffer overflow"));
        } else {
            panic!("Expected buffer overflow error for zero-size buffer");
        }
    }
}
