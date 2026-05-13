use crate::{
    MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait,
    MctpPacketError::{self, HeaderParseError},
    error::{MctpPacketResult, ProtocolError},
    mctp_command_code::MctpControlCommandCode,
    mctp_completion_code::MctpCompletionCode,
};

#[derive(Debug, Default, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpControlHeader {
    pub request_bit: bool,  // bit 7
    pub datagram_bit: bool, // bit 6
    pub instance_id: u8,    // bits 4-0
    pub command_code: MctpControlCommandCode,
    pub completion_code: MctpCompletionCode,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpControl {
    SetEndpointIdRequest([u8; 2]),
    SetEndpointIdResponse([u8; 3]),
    GetEndpointIdRequest,
    GetEndpointIdResponse([u8; 3]),
}

impl MctpMessageHeaderTrait for MctpControlHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        if buffer.len() < 3 {
            return Err(crate::MctpPacketError::SerializeError(
                "buffer too small for mctp control header",
            ));
        }

        check_request_and_completion_code(self.request_bit, self.completion_code)?;

        buffer[0] = (self.request_bit as u8) << 7
            | (self.datagram_bit as u8) << 6
            | (self.instance_id & 0b0001_1111);
        buffer[1] = self.command_code as u8;
        buffer[2] = self.completion_code.into();
        Ok(3)
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        if buffer.len() < 3 {
            return Err(HeaderParseError("buffer too small for mctp control header"));
        }

        let request_bit = buffer[0] & 0b1000_0000 != 0;
        let datagram_bit = buffer[0] & 0b0100_0000 != 0;
        let instance_id = buffer[0] & 0b0001_1111;
        let command_code = MctpControlCommandCode::try_from(buffer[1])
            .map_err(|_| HeaderParseError("invalid mctp command code"))?;
        let completion_code = MctpCompletionCode::try_from(buffer[2])
            .map_err(|_| HeaderParseError("invalid mctp completion code"))?;

        check_request_and_completion_code(request_bit, completion_code)?;

        Ok((
            MctpControlHeader {
                request_bit,
                datagram_bit,
                instance_id,
                command_code,
                completion_code,
            },
            &buffer[3..],
        ))
    }
}

fn check_request_and_completion_code<M: MctpMedium>(
    request_bit: bool,
    completion_code: MctpCompletionCode,
) -> MctpPacketResult<(), M> {
    if request_bit && completion_code != MctpCompletionCode::Success {
        return Err(MctpPacketError::ProtocolError(
            ProtocolError::CompletionCodeOnRequestMessage(completion_code),
        ));
    }
    Ok(())
}

impl<'buf> MctpMessageTrait<'buf> for MctpControl {
    type Header = MctpControlHeader;
    const MESSAGE_TYPE: u8 = 0x00;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        match self {
            Self::SetEndpointIdRequest(data) => copy_and_check_len(buffer, data),
            Self::SetEndpointIdResponse(data) => copy_and_check_len(buffer, data),
            Self::GetEndpointIdRequest => copy_and_check_len(buffer, []),
            Self::GetEndpointIdResponse(data) => copy_and_check_len(buffer, data),
        }
    }

    fn deserialize<M: MctpMedium>(
        header: &Self::Header,
        buffer: &'buf [u8],
    ) -> MctpPacketResult<Self, M> {
        let message = match (header.request_bit, header.command_code) {
            (true, MctpControlCommandCode::SetEndpointId) => {
                Self::SetEndpointIdRequest(try_into_array(buffer)?)
            }
            (true, MctpControlCommandCode::GetEndpointId) => Self::GetEndpointIdRequest,
            (false, MctpControlCommandCode::SetEndpointId) => {
                Self::SetEndpointIdResponse(try_into_array(buffer)?)
            }
            (false, MctpControlCommandCode::GetEndpointId) => {
                Self::GetEndpointIdResponse(try_into_array(buffer)?)
            }
            _ => {
                return Err(HeaderParseError("invalid mctp control command code"));
            }
        };
        Ok(message)
    }
}

fn copy_and_check_len<const N: usize, M: MctpMedium>(
    buffer: &mut [u8],
    data: [u8; N],
) -> MctpPacketResult<usize, M> {
    if buffer.len() < N {
        return Err(crate::MctpPacketError::SerializeError(
            "buffer too small for mctp control message",
        ));
    }
    buffer[..N].copy_from_slice(&data);
    Ok(N)
}

fn try_into_array<const N: usize, M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<[u8; N], M> {
    if buffer.len() < N {
        return Err(HeaderParseError(
            "buffer too small for mctp control message",
        ));
    }
    Ok(buffer[..N].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::ProtocolError, test_util::TestMedium};

    #[test]
    fn header_serialize_deserialize_happy_path() {
        let header = MctpControlHeader {
            request_bit: true,
            datagram_bit: false,
            instance_id: 0b1_1111,
            command_code: MctpControlCommandCode::GetEndpointId,
            completion_code: MctpCompletionCode::Success,
        };

        let mut buf = [0u8; 3];
        let size = header.clone().serialize::<TestMedium>(&mut buf).unwrap();
        assert_eq!(size, 3);
        assert_eq!(
            buf,
            [
                0b1000_0000 | 0b0001_1111, // rq=1, d=0, instance id=0x1F
                MctpControlCommandCode::GetEndpointId as u8,
                u8::from(MctpCompletionCode::Success),
            ]
        );

        let (parsed, rest) = MctpControlHeader::deserialize::<TestMedium>(&buf).unwrap();
        assert_eq!(parsed, header);
        assert_eq!(rest.len(), 0);
    }

    #[test]
    fn header_serialize_error_on_completion_code_in_request() {
        let header = MctpControlHeader {
            request_bit: true,
            datagram_bit: false,
            instance_id: 0,
            command_code: MctpControlCommandCode::SetEndpointId,
            completion_code: MctpCompletionCode::Error,
        };

        let mut buf = [0u8; 3];
        let err = header.serialize::<TestMedium>(&mut buf).unwrap_err();
        match err {
            MctpPacketError::ProtocolError(ProtocolError::CompletionCodeOnRequestMessage(code)) => {
                assert_eq!(code, MctpCompletionCode::Error)
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[rstest::rstest]
    #[case(MctpControlCommandCode::SetEndpointId, false, MctpControl::SetEndpointIdResponse([0xAA, 0xBB, 0xCC]), &[0xAA, 0xBB, 0xCC])]
    #[case(MctpControlCommandCode::SetEndpointId, true, MctpControl::SetEndpointIdRequest([0xAA, 0xBB]), &[0xAA, 0xBB])]
    #[case(MctpControlCommandCode::GetEndpointId, false, MctpControl::GetEndpointIdResponse([0xAA, 0xBB, 0xCC]), &[0xAA, 0xBB, 0xCC])]
    #[case(MctpControlCommandCode::GetEndpointId, true, MctpControl::GetEndpointIdRequest, &[])]
    fn message_serialize_deserialize_happy_path(
        #[case] command_code: MctpControlCommandCode,
        #[case] request_bit: bool,
        #[case] message: MctpControl,
        #[case] expected: &[u8],
    ) {
        let mut buf = [0u8; 1024];
        let size = message.clone().serialize::<TestMedium>(&mut buf).unwrap();
        assert_eq!(size, expected.len());
        assert_eq!(&buf[..size], expected);

        let header = MctpControlHeader {
            request_bit,
            datagram_bit: false,
            instance_id: 0,
            command_code,
            completion_code: MctpCompletionCode::Success,
        };

        let parsed = MctpControl::deserialize::<TestMedium>(&header, &buf).unwrap();
        assert_eq!(parsed, message);
    }

    #[test]
    fn message_deserialize_error_on_invalid_command_for_header() {
        // request message with unsupported command code should error
        let header = MctpControlHeader {
            request_bit: true,
            datagram_bit: false,
            instance_id: 0,
            command_code: MctpControlCommandCode::Reserved,
            completion_code: MctpCompletionCode::Success,
        };

        let err = MctpControl::deserialize::<TestMedium>(&header, &[]).unwrap_err();
        match err {
            MctpPacketError::HeaderParseError(msg) => {
                assert_eq!(msg, "invalid mctp control command code")
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
