use bit_register::bit_register;

use crate::{
    endpoint_id::EndpointId, mctp_message_tag::MctpMessageTag,
    mctp_sequence_number::MctpSequenceNumber,
};

bit_register! {
    #[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct MctpTransportHeader: little_endian u32 {
        pub reserved: u8 => [28:31],
        pub header_version: u8 => [24:27],
        pub destination_endpoint_id: EndpointId => [16:23],
        pub source_endpoint_id: EndpointId => [8:15],
        pub start_of_message: u8 => [7],
        pub end_of_message: u8 => [6],
        pub packet_sequence_number: MctpSequenceNumber => [4:5],
        pub tag_owner: u8 => [3],
        pub message_tag: MctpMessageTag => [0:2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mctp_transport_header_bit_register() {
        let header = MctpTransportHeader::try_from(u32::from_be_bytes([
            0b0000_0001, // reserved, header version (1)
            0b0000_1001, // destination endpoint id (9)
            0b0001_0010, // source endpoint id (18)
            0b0000_0101, /* start of message, end of message, packet sequence number (0), tag
                          * owner, message tag */
        ]))
        .unwrap();

        assert_eq!(
            header,
            MctpTransportHeader {
                reserved: 0b0000,
                header_version: 0b0001,
                destination_endpoint_id: EndpointId::Id(9),
                source_endpoint_id: EndpointId::Id(18),
                start_of_message: 0b0000,
                end_of_message: 0b0000,
                packet_sequence_number: MctpSequenceNumber::new(0),
                tag_owner: 0b0000,
                message_tag: MctpMessageTag::try_from(0b101).unwrap(),
            }
        );
    }
}
