#![no_std]

use embedded_services::ec_type::message::{MessageSerializationError, SerializableMessage};

/// 16-bit variable length
pub type VarLen = u16;

/// Instance ID
pub type InstanceId = u8;

/// Time in milliseconds
pub type Milliseconds = u32;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = u32;

/// Standard MPTF requests expected by the thermal subsystem
#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum ThermalCmd {
    /// EC_THM_GET_TMP = 0x1
    GetTmp = 1,
    /// EC_THM_SET_THRS = 0x2
    SetThrs = 2,
    /// EC_THM_GET_THRS = 0x3
    GetThrs = 3,
    /// EC_THM_SET_SCP = 0x4
    SetScp = 4,
    /// EC_THM_GET_VAR = 0x5
    GetVar = 5,
    /// EC_THM_SET_VAR = 0x6
    SetVar = 6,
}

impl From<&ThermalRequest> for ThermalCmd {
    fn from(request: &ThermalRequest) -> Self {
        match request {
            ThermalRequest::ThermalGetTmpRequest { .. } => ThermalCmd::GetTmp,
            ThermalRequest::ThermalSetThrsRequest { .. } => ThermalCmd::SetThrs,
            ThermalRequest::ThermalGetThrsRequest { .. } => ThermalCmd::GetThrs,
            ThermalRequest::ThermalSetScpRequest { .. } => ThermalCmd::SetScp,
            ThermalRequest::ThermalGetVarRequest { .. } => ThermalCmd::GetVar,
            ThermalRequest::ThermalSetVarRequest { .. } => ThermalCmd::SetVar,
        }
    }
}

impl From<&ThermalResponse> for ThermalCmd {
    fn from(response: &ThermalResponse) -> Self {
        match response {
            ThermalResponse::ThermalGetTmpResponse { .. } => ThermalCmd::GetTmp,
            ThermalResponse::ThermalSetThrsResponse => ThermalCmd::SetThrs,
            ThermalResponse::ThermalGetThrsResponse { .. } => ThermalCmd::GetThrs,
            ThermalResponse::ThermalSetScpResponse => ThermalCmd::SetScp,
            ThermalResponse::ThermalGetVarResponse { .. } => ThermalCmd::GetVar,
            ThermalResponse::ThermalSetVarResponse => ThermalCmd::SetVar,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThermalRequest {
    ThermalGetTmpRequest {
        instance_id: u8,
    },
    ThermalSetThrsRequest {
        instance_id: u8,
        timeout: Milliseconds,
        low: DeciKelvin,
        high: DeciKelvin,
    },
    ThermalGetThrsRequest {
        instance_id: u8,
    },
    ThermalSetScpRequest {
        instance_id: u8,
        policy_id: u32,
        acoustic_lim: u32,
        power_lim: u32,
    },
    ThermalGetVarRequest {
        instance_id: u8,
        len: VarLen, // TODO why is there a len here? as far as I can tell we're always discarding it, and I think values are only u32?
        var_uuid: uuid::Bytes,
    },
    ThermalSetVarRequest {
        instance_id: u8,
        len: VarLen, // TODO why is there a len here? as far as I can tell we're always discarding it, and I think values are only u32?
        var_uuid: uuid::Bytes,
        set_var: u32,
    },
}

// TODO this is essentially a hand-written reinterpret_cast - can we codegen some of this instead?
impl SerializableMessage for ThermalRequest {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to serialize requests on the EC side",
        ))
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match ThermalCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                ThermalCmd::GetTmp => Self::ThermalGetTmpRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                },
                ThermalCmd::SetThrs => Self::ThermalSetThrsRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                    timeout: safe_get_dword(buffer, 1)?,
                    low: safe_get_dword(buffer, 5)?,
                    high: safe_get_dword(buffer, 9)?,
                },
                ThermalCmd::GetThrs => Self::ThermalGetThrsRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                },
                ThermalCmd::SetScp => Self::ThermalSetScpRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                    policy_id: safe_get_dword(buffer, 1)?,
                    acoustic_lim: safe_get_dword(buffer, 5)?,
                    power_lim: safe_get_dword(buffer, 9)?,
                },
                ThermalCmd::GetVar => Self::ThermalGetVarRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                    len: safe_get_u16(buffer, 1)?,
                    var_uuid: safe_get_uuid(buffer, 3)?,
                },
                ThermalCmd::SetVar => Self::ThermalSetVarRequest {
                    instance_id: safe_get_u8(buffer, 0)?,
                    len: safe_get_u16(buffer, 1)?,
                    var_uuid: safe_get_uuid(buffer, 3)?,
                    set_var: safe_get_dword(buffer, 19)?,
                },
            },
        )
    }

    fn discriminant(&self) -> u16 {
        let cmd: ThermalCmd = self.into();
        cmd.into()
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThermalResponse {
    ThermalGetTmpResponse {
        temperature: DeciKelvin,
    },
    ThermalSetThrsResponse,
    ThermalGetThrsResponse {
        timeout: Milliseconds,
        low: DeciKelvin,
        high: DeciKelvin,
    },
    ThermalSetScpResponse,
    ThermalGetVarResponse {
        val: u32,
    },
    ThermalSetVarResponse,
}

impl SerializableMessage for ThermalResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::ThermalGetTmpResponse { temperature } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(temperature));

                Ok(4)
            }
            Self::ThermalGetThrsResponse { timeout, low, high } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(timeout));
                buffer
                    .get_mut(4..8)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(low));
                buffer
                    .get_mut(8..12)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(high));

                Ok(12)
            }

            Self::ThermalGetVarResponse { val } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(val));
                Ok(4)
            }
            Self::ThermalSetVarResponse | Self::ThermalSetScpResponse | Self::ThermalSetThrsResponse => Ok(0),
        }
    }

    fn deserialize(_discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to deserialize responses on the EC side",
        ))
    }

    fn discriminant(&self) -> u16 {
        ThermalCmd::from(self).into()
    }
}

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum ThermalError {
    InvalidParameter = 1,
    UnsupportedRevision = 2,
    HardwareError = 3,
}

impl SerializableMessage for ThermalError {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::UnsupportedRevision | Self::InvalidParameter | Self::HardwareError => Ok(0),
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

pub type ThermalResult = Result<ThermalResponse, ThermalError>;

fn safe_get_u8(buffer: &[u8], index: usize) -> Result<u8, MessageSerializationError> {
    buffer
        .get(index)
        .copied()
        .ok_or(MessageSerializationError::BufferTooSmall)
}

fn safe_get_u16(buffer: &[u8], index: usize) -> Result<u16, MessageSerializationError> {
    let bytes = buffer
        .get(index..index + 2)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)?;
    Ok(u16::from_le_bytes(bytes))
}

fn safe_get_dword(buffer: &[u8], index: usize) -> Result<u32, MessageSerializationError> {
    let bytes = buffer
        .get(index..index + 4)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)?;
    Ok(u32::from_le_bytes(bytes))
}

fn safe_get_uuid(buffer: &[u8], index: usize) -> Result<uuid::Bytes, MessageSerializationError> {
    buffer
        .get(index..index + 16)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)
}
