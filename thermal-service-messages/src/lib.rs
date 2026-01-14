#![no_std]

use embedded_services::relay::{MessageSerializationError, SerializableMessage};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, LE, U16, U32};

/// 16-bit variable length
pub type VarLen = U16<LE>;

/// Instance ID
pub type InstanceId = u8;

/// Time in milliseconds
pub type Milliseconds = U32<LE>;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = U32<LE>;

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
            ThermalRequest::ThermalGetTmpRequest(_) => ThermalCmd::GetTmp,
            ThermalRequest::ThermalSetThrsRequest(_) => ThermalCmd::SetThrs,
            ThermalRequest::ThermalGetThrsRequest(_) => ThermalCmd::GetThrs,
            ThermalRequest::ThermalSetScpRequest(_) => ThermalCmd::SetScp,
            ThermalRequest::ThermalGetVarRequest(_) => ThermalCmd::GetVar,
            ThermalRequest::ThermalSetVarRequest(_) => ThermalCmd::SetVar,
        }
    }
}

impl From<&ThermalResponse> for ThermalCmd {
    fn from(response: &ThermalResponse) -> Self {
        match response {
            ThermalResponse::ThermalGetTmpResponse(_) => ThermalCmd::GetTmp,
            ThermalResponse::ThermalSetThrsResponse => ThermalCmd::SetThrs,
            ThermalResponse::ThermalGetThrsResponse(_) => ThermalCmd::GetThrs,
            ThermalResponse::ThermalSetScpResponse => ThermalCmd::SetScp,
            ThermalResponse::ThermalGetVarResponse(_) => ThermalCmd::GetVar,
            ThermalResponse::ThermalSetVarResponse => ThermalCmd::SetVar,
        }
    }
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ThermalGetTmpRequest {
    pub instance_id: u8,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalSetThrsRequest {
    pub instance_id: u8,
    pub timeout: Milliseconds,
    pub low: DeciKelvin,
    pub high: DeciKelvin,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ThermalGetThrsRequest {
    pub instance_id: u8,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalSetScpRequest {
    pub instance_id: u8,
    pub policy_id: U32<LE>,
    pub acoustic_lim: U32<LE>,
    pub power_lim: U32<LE>,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalGetVarRequest {
    pub instance_id: u8,
    pub len: VarLen, // TODO why is there a len here? as far as I can tell we're always discarding it, and I think values are only u32?
    pub var_uuid: uuid::Bytes,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalSetVarRequest {
    pub instance_id: u8,
    pub len: VarLen, // TODO why is there a len here? as far as I can tell we're always discarding it, and I think values are only u32?
    pub var_uuid: uuid::Bytes,
    pub set_var: U32<LE>,
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThermalRequest {
    ThermalGetTmpRequest(ThermalGetTmpRequest),
    ThermalSetThrsRequest(ThermalSetThrsRequest),
    ThermalGetThrsRequest(ThermalGetThrsRequest),
    ThermalSetScpRequest(ThermalSetScpRequest),
    ThermalGetVarRequest(ThermalGetVarRequest),
    ThermalSetVarRequest(ThermalSetVarRequest),
}

impl SerializableMessage for ThermalRequest {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::ThermalGetTmpRequest(req) => serialize_inner(req, buffer),
            Self::ThermalSetThrsRequest(req) => serialize_inner(req, buffer),
            Self::ThermalGetThrsRequest(req) => serialize_inner(req, buffer),
            Self::ThermalSetScpRequest(req) => serialize_inner(req, buffer),
            Self::ThermalGetVarRequest(req) => serialize_inner(req, buffer),
            Self::ThermalSetVarRequest(req) => serialize_inner(req, buffer),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        let cmd = ThermalCmd::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?;

        Ok(match cmd {
            ThermalCmd::GetTmp => Self::ThermalGetTmpRequest(deserialize_inner(buffer)?),
            ThermalCmd::SetThrs => Self::ThermalSetThrsRequest(deserialize_inner(buffer)?),
            ThermalCmd::GetThrs => Self::ThermalGetThrsRequest(deserialize_inner(buffer)?),
            ThermalCmd::SetScp => Self::ThermalSetScpRequest(deserialize_inner(buffer)?),
            ThermalCmd::GetVar => Self::ThermalGetVarRequest(deserialize_inner(buffer)?),
            ThermalCmd::SetVar => Self::ThermalSetVarRequest(deserialize_inner(buffer)?),
        })
    }

    fn discriminant(&self) -> u16 {
        let cmd: ThermalCmd = self.into();
        cmd.into()
    }
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalGetTmpResponse {
    pub temperature: DeciKelvin,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalGetThrsResponse {
    pub timeout: Milliseconds,
    pub low: DeciKelvin,
    pub high: DeciKelvin,
}

#[derive(PartialEq, Clone, Copy, Debug, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct ThermalGetVarResponse {
    pub val: U32<LE>,
}

#[derive(PartialEq, Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThermalResponse {
    ThermalGetTmpResponse(ThermalGetTmpResponse),
    ThermalSetThrsResponse,
    ThermalGetThrsResponse(ThermalGetThrsResponse),
    ThermalSetScpResponse,
    ThermalGetVarResponse(ThermalGetVarResponse),
    ThermalSetVarResponse,
}

impl SerializableMessage for ThermalResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::ThermalGetTmpResponse(resp) => serialize_inner(resp, buffer),
            Self::ThermalGetThrsResponse(resp) => serialize_inner(resp, buffer),
            Self::ThermalGetVarResponse(resp) => serialize_inner(resp, buffer),
            Self::ThermalSetVarResponse | Self::ThermalSetThrsResponse | Self::ThermalSetScpResponse => Ok(0),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        let cmd = ThermalCmd::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?;

        Ok(match cmd {
            ThermalCmd::GetTmp => Self::ThermalGetTmpResponse(deserialize_inner(buffer)?),
            ThermalCmd::GetThrs => Self::ThermalGetThrsResponse(deserialize_inner(buffer)?),
            ThermalCmd::GetVar => Self::ThermalGetVarResponse(deserialize_inner(buffer)?),
            ThermalCmd::SetThrs => Self::ThermalSetThrsResponse,
            ThermalCmd::SetScp => Self::ThermalSetScpResponse,
            ThermalCmd::SetVar => Self::ThermalSetVarResponse,
        })
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
        Ok(0)
    }

    fn deserialize(discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        ThermalError::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))
    }

    fn discriminant(&self) -> u16 {
        (*self).into()
    }
}

pub type ThermalResult = Result<ThermalResponse, ThermalError>;

#[inline(always)]
fn serialize_inner<T: IntoBytes + Immutable>(req: T, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
    req.write_to_prefix(buffer)
        .map_err(|_| MessageSerializationError::BufferTooSmall)?;
    Ok(req.as_bytes().len())
}

#[inline(always)]
fn deserialize_inner<T: FromBytes>(buffer: &[u8]) -> Result<T, MessageSerializationError> {
    Ok(T::read_from_prefix(buffer)
        .map_err(|_| MessageSerializationError::BufferTooSmall)?
        .0)
}

// NOTE: zerocopy::byteorder::UN types unfortunately don't implement `defmt::Format`, so the structs
// can't derive it. Thus we have to manually implement it.
//
// Revisit: Upstream defmt support to zerocopy?
#[cfg(feature = "defmt")]
impl defmt::Format for ThermalSetThrsRequest {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "ThermalSetThrsRequest {{ instance_id: {}, timeout: {}, low: {}, high: {} }}",
            self.instance_id,
            self.timeout.get(),
            self.low.get(),
            self.high.get(),
        );
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalSetScpRequest {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "ThermalSetScpRequest {{ instance_id: {}, policy_id: {}, acoustic_lim: {}, power_lim: {} }}",
            self.instance_id,
            self.policy_id.get(),
            self.acoustic_lim.get(),
            self.power_lim.get(),
        );
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalGetVarRequest {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "ThermalGetVarRequest {{ instance_id: {}, len: {}, var_uuid: {=[u8; 16]} }}",
            self.instance_id,
            self.len.get(),
            self.var_uuid,
        );
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalSetVarRequest {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "ThermalSetVarRequest {{ instance_id: {}, len: {}, var_uuid: {=[u8; 16]}, set_var: {} }}",
            self.instance_id,
            self.len.get(),
            self.var_uuid,
            self.set_var.get(),
        );
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalGetTmpResponse {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(f, "ThermalGetTmpResponse {{ temperature: {} }}", self.temperature.get());
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalGetThrsResponse {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "ThermalGetThrsResponse {{ timeout: {}, low: {}, high: {} }}",
            self.timeout.get(),
            self.low.get(),
            self.high.get(),
        );
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ThermalGetVarResponse {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(f, "ThermalGetVarResponse {{ val: {} }}", self.val.get());
    }
}
