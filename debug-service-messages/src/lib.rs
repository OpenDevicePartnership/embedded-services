#![no_std]
use embedded_services::relay::{MessageSerializationError, SerializableMessage};

/// Standard Debug Service Log Buffer Size
pub const STD_DEBUG_BUF_SIZE: usize = 128;

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// ODP Specific Debug Commands
enum DebugCmd {
    /// Get buffer of debug messages, if available.
    /// Can be used to poll debug messages.
    GetMsgs = 1,
}

impl From<&DebugRequest> for DebugCmd {
    fn from(request: &DebugRequest) -> Self {
        match request {
            DebugRequest::DebugGetMsgsRequest => DebugCmd::GetMsgs,
        }
    }
}

impl From<&DebugResponse> for DebugCmd {
    fn from(response: &DebugResponse) -> Self {
        match response {
            DebugResponse::DebugGetMsgsResponse { .. } => DebugCmd::GetMsgs,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DebugRequest {
    DebugGetMsgsRequest,
}

impl SerializableMessage for DebugRequest {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to serialize requests on the EC side",
        ))
    }
    fn deserialize(discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match DebugCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                DebugCmd::GetMsgs => Self::DebugGetMsgsRequest,
            },
        )
    }

    fn discriminant(&self) -> u16 {
        let cmd: DebugCmd = self.into();
        cmd.into()
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DebugResponse {
    DebugGetMsgsResponse { debug_buf: [u8; STD_DEBUG_BUF_SIZE] },
}

impl SerializableMessage for DebugResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::DebugGetMsgsResponse { debug_buf } => {
                buffer
                    .get_mut(..debug_buf.len())
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&debug_buf);
                Ok(debug_buf.len())
            }
        }
    }
    fn deserialize(_discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to serialize requests on the EC side",
        ))
    }

    fn discriminant(&self) -> u16 {
        DebugCmd::from(self).into()
    }
}

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum DebugError {
    UnspecifiedFailure = 1,
}

impl SerializableMessage for DebugError {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::UnspecifiedFailure => Ok(0),
        }
    }

    fn deserialize(_discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to deserialize responses on the EC side",
        ))
    }

    fn discriminant(&self) -> u16 {
        (*self).into()
    }
}

pub type DebugResult = Result<DebugResponse, DebugError>;
