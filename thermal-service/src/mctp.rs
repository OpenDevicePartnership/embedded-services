use crate::mptf::*;

pub const CURRENT_VERSION: u8 = 1;

/// Raw MCTP Payload
pub type Payload = ([u8; 69], usize);

/// MCTP Payload Error
pub struct PayloadError {
    command: u8,
    error: Status,
}

impl PayloadError {
    fn new(command: u8, error: Status) -> Self {
        Self { command, error }
    }
}

impl TryFrom<Payload> for Request {
    type Error = PayloadError;
    fn try_from(payload: Payload) -> Result<Self, Self::Error> {
        let payload_len = payload.1;
        let (version, _rsvd, _status, command, data) = (
            payload.0[0],
            payload.0[1],
            payload.0[2],
            payload.0[3],
            &payload.0[4..payload_len],
        );
        if version != CURRENT_VERSION {
            return Err(PayloadError::new(command, Status::UnsupportedRevision));
        }

        match command {
            1 => Ok(Request::GetTmp(data[0])),

            2 => Ok(Request::SetThrs(
                data[0],
                Milliseconds::from_le_bytes(
                    data[1..5]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                DeciKelvin::from_le_bytes(
                    data[5..9]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                DeciKelvin::from_le_bytes(
                    data[9..13]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
            )),

            3 => Ok(Request::GetThrs(data[0])),

            4 => Ok(Request::SetScp(
                data[0],
                Dword::from_le_bytes(
                    data[1..5]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                Dword::from_le_bytes(
                    data[5..9]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                Dword::from_le_bytes(
                    data[9..13]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
            )),

            5 => Ok(Request::GetVar(
                data[0],
                VarLen::from_le_bytes(
                    data[1..3]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                data[3..19]
                    .try_into()
                    .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
            )),

            6 => Ok(Request::SetVar(
                data[0],
                VarLen::from_le_bytes(
                    data[1..3]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
                data[3..19]
                    .try_into()
                    .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                Dword::from_le_bytes(
                    data[19..23]
                        .try_into()
                        .map_err(|_| PayloadError::new(command, Status::InvalidParameter))?,
                ),
            )),

            _ => Err(PayloadError::new(command, Status::InvalidParameter)),
        }
    }
}

impl From<Response> for Payload {
    fn from(response: Response) -> Self {
        let mut payload = [0; 69];
        payload[0] = CURRENT_VERSION; // Version
        payload[1] = 0; // Reserved
        payload[2] = u8::from(response.status); // Status
        payload[3] = response.data.into(); // Command

        let (header, data) = payload.split_at_mut(4);
        let header_len = header.len();

        let data_len = match response.data {
            ResponseData::GetTmp(temp) => {
                data[0..4].copy_from_slice(&temp.to_le_bytes());
                4
            }
            ResponseData::SetThrs(status) => {
                data[0..4].copy_from_slice(&u32::from(status).to_le_bytes());
                4
            }
            ResponseData::GetThrs(status, timeout, low_dk, high_dk) => {
                data[0..4].copy_from_slice(&u32::from(status).to_le_bytes());
                data[4..8].copy_from_slice(&timeout.to_le_bytes());
                data[8..12].copy_from_slice(&low_dk.to_le_bytes());
                data[12..16].copy_from_slice(&high_dk.to_le_bytes());
                16
            }
            ResponseData::SetScp(status) => {
                data[0..4].copy_from_slice(&u32::from(status).to_le_bytes());
                4
            }
            ResponseData::GetVar(status, value) => {
                data[0..4].copy_from_slice(&u32::from(status).to_le_bytes());
                data[4..8].copy_from_slice(&value.to_le_bytes());
                8
            }
            ResponseData::SetVar(status) => {
                data[0..4].copy_from_slice(&u32::from(status).to_le_bytes());
                4
            }
        };

        (payload, header_len + data_len)
    }
}

impl From<PayloadError> for Payload {
    fn from(mctp_error: PayloadError) -> Self {
        let mut payload = [0; 69];
        payload[0] = CURRENT_VERSION; // Version
        payload[1] = 0; // Reserved
        payload[2] = u8::from(mctp_error.error); // Status
        payload[3] = mctp_error.command; // Command
        (payload, 4)
    }
}
