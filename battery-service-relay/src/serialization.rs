use battery_service_interface::*;
use embedded_services::relay::{MessageSerializationError, SerializableMessage};

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// ACPI Battery Methods
enum BatteryCmd {
    /// Battery Information eXtended
    GetBix = 1,
    /// Battery Status
    GetBst = 2,
    /// Power Source
    GetPsr = 3,
    /// Power source InFormation
    GetPif = 4,
    /// Battery Power State
    GetBps = 5,
    /// Battery Trip Point
    SetBtp = 6,
    /// Battery Power Threshold
    SetBpt = 7,
    /// Battery Power Characteristics
    GetBpc = 8,
    /// Battery Maintenance Control
    SetBmc = 9,
    /// Battery Maintenance Data
    GetBmd = 10,
    /// Battery Charge Time
    GetBct = 11,
    /// Battery Time
    GetBtm = 12,
    /// Battery Measurement Sampling Time
    SetBms = 13,
    /// Battery Measurement Averaging Interval
    SetBma = 14,
    /// Device Status
    GetSta = 15,
}

impl From<&AcpiBatteryRequest> for BatteryCmd {
    fn from(request: &AcpiBatteryRequest) -> Self {
        match request {
            AcpiBatteryRequest::GetBix { .. } => BatteryCmd::GetBix,
            AcpiBatteryRequest::GetBst { .. } => BatteryCmd::GetBst,
            AcpiBatteryRequest::GetPsr { .. } => BatteryCmd::GetPsr,
            AcpiBatteryRequest::GetPif { .. } => BatteryCmd::GetPif,
            AcpiBatteryRequest::GetBps { .. } => BatteryCmd::GetBps,
            AcpiBatteryRequest::SetBtp { .. } => BatteryCmd::SetBtp,
            AcpiBatteryRequest::SetBpt { .. } => BatteryCmd::SetBpt,
            AcpiBatteryRequest::GetBpc { .. } => BatteryCmd::GetBpc,
            AcpiBatteryRequest::SetBmc { .. } => BatteryCmd::SetBmc,
            AcpiBatteryRequest::GetBmd { .. } => BatteryCmd::GetBmd,
            AcpiBatteryRequest::GetBct { .. } => BatteryCmd::GetBct,
            AcpiBatteryRequest::GetBtm { .. } => BatteryCmd::GetBtm,
            AcpiBatteryRequest::SetBms { .. } => BatteryCmd::SetBms,
            AcpiBatteryRequest::SetBma { .. } => BatteryCmd::SetBma,
            AcpiBatteryRequest::GetSta { .. } => BatteryCmd::GetSta,
        }
    }
}

impl From<&AcpiBatteryResponse> for BatteryCmd {
    fn from(response: &AcpiBatteryResponse) -> Self {
        match response {
            AcpiBatteryResponse::GetBix { .. } => BatteryCmd::GetBix,
            AcpiBatteryResponse::GetBst { .. } => BatteryCmd::GetBst,
            AcpiBatteryResponse::GetPsr { .. } => BatteryCmd::GetPsr,
            AcpiBatteryResponse::GetPif { .. } => BatteryCmd::GetPif,
            AcpiBatteryResponse::GetBps { .. } => BatteryCmd::GetBps,
            AcpiBatteryResponse::SetBtp { .. } => BatteryCmd::SetBtp,
            AcpiBatteryResponse::SetBpt { .. } => BatteryCmd::SetBpt,
            AcpiBatteryResponse::GetBpc { .. } => BatteryCmd::GetBpc,
            AcpiBatteryResponse::SetBmc { .. } => BatteryCmd::SetBmc,
            AcpiBatteryResponse::GetBmd { .. } => BatteryCmd::GetBmd,
            AcpiBatteryResponse::GetBct { .. } => BatteryCmd::GetBct,
            AcpiBatteryResponse::GetBtm { .. } => BatteryCmd::GetBtm,
            AcpiBatteryResponse::SetBms { .. } => BatteryCmd::SetBms,
            AcpiBatteryResponse::SetBma { .. } => BatteryCmd::SetBma,
            AcpiBatteryResponse::GetSta { .. } => BatteryCmd::GetSta,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// ACPI battery device message responses as defined in ACPI spec version 6.4, section 10.2
pub enum AcpiBatteryResponse {
    /// Extended battery information. Analogous to the return value of the _BIX method.
    GetBix { bix: BixFixedStrings },

    /// Battery status. Analogous to the return value of the _BST method.
    GetBst { bst: BstReturn },

    /// Power source in use. Analogous to the return value of the _PSR method.
    GetPsr { psr: PsrReturn },

    /// Power source information. Analogous to the return value of the _PIF method.
    GetPif { pif: PifFixedStrings },

    /// Battery power state. Analogous to the return value of the _BPS method.
    GetBps { bps: Bps },

    /// Result of setting a battery trip point. Analogous to the _BTP method. Semantically equivalent to ().
    SetBtp {},

    /// Result of setting a battery power threshold. Analogous to the _BPT method. Semantically equivalent to ().
    SetBpt {},

    /// Battery power characteristics. Analogous to the return value of the _BPC method.
    GetBpc { bpc: Bpc },

    /// Result of performing a battery maintenance control operation. Analogous to the return value of the _BMC method. Semantically equivalent to ().
    SetBmc {},

    /// Battery maintenance data. Analogous to the return value of the _BMD method.
    GetBmd { bmd: Bmd },

    /// Battery charge time. Analogous to the return value of the _BCT method.
    GetBct { bct_response: BctReturnResult },

    /// Battery time to empty. Analogous to the return value of the _BTM method.
    GetBtm { btm_response: BtmReturnResult },

    /// Result of setting the battery measurement sampling time. Analogous to the _BMS method.
    SetBms { status: u32 },

    /// Result of setting the battery measurement averaging interval. Analogous to the _BMA method.
    SetBma { status: u32 },

    /// Battery device status. Analogous to the return value of the _STA method.
    GetSta { sta: StaReturn },
}

impl SerializableMessage for AcpiBatteryResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::GetBix { bix } => bix_to_bytes(bix, buffer),
            Self::GetBst { bst } => Ok(safe_put_dword(buffer, 0, bst.battery_state.bits())?
                + safe_put_dword(buffer, 4, bst.battery_present_rate)?
                + safe_put_dword(buffer, 8, bst.battery_remaining_capacity)?
                + safe_put_dword(buffer, 12, bst.battery_present_voltage)?),
            Self::GetPsr { psr } => safe_put_dword(buffer, 0, psr.power_source.into()),

            Self::GetPif { pif } => pif_to_bytes(pif, buffer),
            Self::GetBps { bps } => Ok(safe_put_dword(buffer, 0, bps.revision)?
                + safe_put_dword(buffer, 4, bps.instantaneous_peak_power_level)?
                + safe_put_dword(buffer, 8, bps.instantaneous_peak_power_period)?
                + safe_put_dword(buffer, 12, bps.sustainable_peak_power_level)?
                + safe_put_dword(buffer, 16, bps.sustainable_peak_power_period)?),
            Self::SetBtp {} => Ok(0),
            Self::SetBpt {} => Ok(0),
            Self::GetBpc { bpc } => Ok(safe_put_dword(buffer, 0, bpc.revision)?
                + safe_put_dword(buffer, 4, bpc.power_threshold_support.bits())?
                + safe_put_dword(buffer, 8, bpc.max_instantaneous_peak_power_threshold)?
                + safe_put_dword(buffer, 12, bpc.max_sustainable_peak_power_threshold)?),
            Self::SetBmc {} => Ok(0),
            Self::GetBmd { bmd } => Ok(safe_put_dword(buffer, 0, bmd.status_flags.bits())?
                + safe_put_dword(buffer, 4, bmd.capability_flags.bits())?
                + safe_put_dword(buffer, 8, bmd.recalibrate_count)?
                + safe_put_dword(buffer, 12, bmd.quick_recalibrate_time)?
                + safe_put_dword(buffer, 16, bmd.slow_recalibrate_time)?),
            Self::GetBct { bct_response } => safe_put_dword(buffer, 0, bct_response.into()),
            Self::GetBtm { btm_response } => safe_put_dword(buffer, 0, btm_response.into()),
            Self::SetBms { status } => safe_put_dword(buffer, 0, status),
            Self::SetBma { status } => safe_put_dword(buffer, 0, status),
            Self::GetSta { sta } => safe_put_dword(buffer, 0, sta.bits()),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match BatteryCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                BatteryCmd::GetBix => Self::GetBix {
                    bix: bix_from_bytes(buffer)?,
                },
                BatteryCmd::GetBst => {
                    let bst = BstReturn {
                        battery_state: BatteryState::from_bits(safe_get_dword(buffer, 0)?)
                            .ok_or(MessageSerializationError::InvalidPayload("Invalid BatteryState"))?,
                        battery_present_rate: safe_get_dword(buffer, 4)?,
                        battery_remaining_capacity: safe_get_dword(buffer, 8)?,
                        battery_present_voltage: safe_get_dword(buffer, 12)?,
                    };
                    Self::GetBst { bst }
                }
                BatteryCmd::GetPsr => Self::GetPsr {
                    psr: PsrReturn {
                        power_source: safe_get_dword(buffer, 0)?
                            .try_into()
                            .map_err(|_| MessageSerializationError::InvalidPayload("Invalid PowerSource"))?,
                    },
                },
                BatteryCmd::GetPif => Self::GetPif {
                    pif: pif_from_bytes(buffer)?,
                },
                BatteryCmd::GetBps => Self::GetBps {
                    bps: Bps {
                        revision: safe_get_dword(buffer, 0)?,
                        instantaneous_peak_power_level: safe_get_dword(buffer, 4)?,
                        instantaneous_peak_power_period: safe_get_dword(buffer, 8)?,
                        sustainable_peak_power_level: safe_get_dword(buffer, 12)?,
                        sustainable_peak_power_period: safe_get_dword(buffer, 16)?,
                    },
                },
                BatteryCmd::SetBtp => Self::SetBtp {},
                BatteryCmd::SetBpt => Self::SetBpt {},
                BatteryCmd::GetBpc => Self::GetBpc {
                    bpc: Bpc {
                        revision: safe_get_dword(buffer, 0)?,
                        power_threshold_support: PowerThresholdSupport::from_bits(safe_get_dword(buffer, 4)?)
                            .ok_or(MessageSerializationError::InvalidPayload("Invalid BpcThresholdSupport"))?,
                        max_instantaneous_peak_power_threshold: safe_get_dword(buffer, 8)?,
                        max_sustainable_peak_power_threshold: safe_get_dword(buffer, 12)?,
                    },
                },
                BatteryCmd::SetBmc => Self::SetBmc {},
                BatteryCmd::GetBmd => Self::GetBmd {
                    bmd: Bmd {
                        status_flags: BmdStatusFlags::from_bits(safe_get_dword(buffer, 0)?)
                            .ok_or(MessageSerializationError::InvalidPayload("Invalid BmdStatusFlags"))?,
                        capability_flags: BmdCapabilityFlags::from_bits(safe_get_dword(buffer, 4)?)
                            .ok_or(MessageSerializationError::InvalidPayload("Invalid BmdCapabilityFlags"))?,
                        recalibrate_count: safe_get_dword(buffer, 8)?,
                        quick_recalibrate_time: safe_get_dword(buffer, 12)?,
                        slow_recalibrate_time: safe_get_dword(buffer, 16)?,
                    },
                },
                BatteryCmd::GetBct => Self::GetBct {
                    bct_response: safe_get_dword(buffer, 0)?.into(),
                },
                BatteryCmd::GetBtm => Self::GetBtm {
                    btm_response: safe_get_dword(buffer, 0)?.into(),
                },
                BatteryCmd::SetBms => Self::SetBms {
                    status: safe_get_dword(buffer, 0)?,
                },
                BatteryCmd::SetBma => Self::SetBma {
                    status: safe_get_dword(buffer, 0)?,
                },
                BatteryCmd::GetSta => Self::GetSta {
                    sta: StaReturn::from_bits(safe_get_dword(buffer, 0)?)
                        .ok_or(MessageSerializationError::InvalidPayload("Invalid STA flags"))?,
                },
            },
        )
    }

    fn discriminant(&self) -> u16 {
        BatteryCmd::from(self).into()
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AcpiBatteryRequest {
    /// Queries extended battery information. Analogous to ACPI's _BIX method.
    GetBix { battery_id: u8 },

    /// Queries battery status. Analogous to ACPI's _BST method.
    GetBst { battery_id: u8 },

    /// Queries the current power source. Analogous to ACPI's _PSR method.
    GetPsr { battery_id: u8 },

    /// Queries information about the battery's power source. Analogous to ACPI's _PIF method.
    GetPif { battery_id: u8 },

    /// Queries information about the current power delivery capabilities of the battery. Analogous to ACPI's _BPS method.
    GetBps { battery_id: u8 },

    /// Sets a battery trip point. Analogous to ACPI's _BTP method.
    SetBtp { battery_id: u8, btp: Btp },

    /// Sets a battery power threshold. Analogous to ACPI's _BPT method.
    SetBpt { battery_id: u8, bpt: Bpt },

    /// Queries the current power characteristics of the battery. Analogous to ACPI's _BPC method.
    GetBpc { battery_id: u8 },

    /// Performs a battery maintenance control operation. Analogous to ACPI's _BMC method.
    SetBmc { battery_id: u8, bmc: Bmc },

    /// Queries battery maintenance data. Analogous to ACPI's _BMD method.
    GetBmd { battery_id: u8 },

    /// Queries the estimated time remaining to charge the battery to the specified level. Analogous to ACPI's _BCT method.
    GetBct { battery_id: u8, bct: Bct },

    /// Queries the estimated time remaining until the battery is discharged to the specified level. Analogous to ACPI's _BTM method.
    GetBtm { battery_id: u8, btm: Btm },

    /// Sets the sampling time of battery measurements in milliseconds. Analogous to ACPI's _BMS method.
    SetBms { battery_id: u8, bms: Bms },

    /// Sets the averaging interval of battery measurements in milliseconds. Analogous to ACPI's _BMA method.
    SetBma { battery_id: u8, bma: Bma },

    /// Queries the current status of the battery device. Analogous to ACPI's _STA method.
    GetSta { battery_id: u8 },
}

impl SerializableMessage for AcpiBatteryRequest {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::GetBix { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::GetBst { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::GetPsr { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::GetPif { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::GetBps { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::SetBtp { battery_id, btp } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, btp.trip_point)?)
            }
            Self::SetBpt { battery_id, bpt } => Ok(safe_put_u8(buffer, 0, battery_id)?
                + safe_put_dword(buffer, 1, bpt.revision)?
                + safe_put_dword(buffer, 5, bpt.threshold_id as u32)?
                + safe_put_dword(buffer, 9, bpt.threshold_value)?),
            Self::GetBpc { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::SetBmc { battery_id, bmc } => {
                Ok(safe_put_u8(buffer, 0, battery_id)?
                    + safe_put_dword(buffer, 1, bmc.maintenance_control_flags.bits())?)
            }
            Self::GetBmd { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::GetBct { battery_id, bct } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bct.charge_level_percent)?)
            }
            Self::GetBtm { battery_id, btm } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, btm.discharge_rate)?)
            }
            Self::SetBms { battery_id, bms } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bms.sampling_time_ms)?)
            }
            Self::SetBma { battery_id, bma } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bma.averaging_interval_ms)?)
            }
            Self::GetSta { battery_id } => safe_put_u8(buffer, 0, battery_id),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match BatteryCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                BatteryCmd::GetBix => Self::GetBix {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBst => Self::GetBst {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetPsr => Self::GetPsr {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetPif => Self::GetPif {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBps => Self::GetBps {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::SetBtp => Self::SetBtp {
                    battery_id: safe_get_u8(buffer, 0)?,
                    btp: Btp {
                        trip_point: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBpt => Self::SetBpt {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bpt: Bpt {
                        revision: safe_get_dword(buffer, 1)?,
                        threshold_id: safe_get_dword(buffer, 5)?
                            .try_into()
                            .map_err(|_| MessageSerializationError::InvalidPayload("Invalid ThresholdId"))?,
                        threshold_value: safe_get_dword(buffer, 9)?,
                    },
                },
                BatteryCmd::GetBpc => Self::GetBpc {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::SetBmc => Self::SetBmc {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bmc: Bmc {
                        maintenance_control_flags: BmcControlFlags::from_bits_retain(safe_get_dword(buffer, 1)?),
                    },
                },
                BatteryCmd::GetBmd => Self::GetBmd {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBct => Self::GetBct {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bct: Bct {
                        charge_level_percent: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::GetBtm => Self::GetBtm {
                    battery_id: safe_get_u8(buffer, 0)?,
                    btm: Btm {
                        discharge_rate: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBms => Self::SetBms {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bms: Bms {
                        sampling_time_ms: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBma => Self::SetBma {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bma: Bma {
                        averaging_interval_ms: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::GetSta => Self::GetSta {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
            },
        )
    }

    fn discriminant(&self) -> u16 {
        BatteryCmd::from(self).into()
    }
}

/// Serializable result type for battery operations.
pub type AcpiBatteryResult = Result<AcpiBatteryResponse, AcpiBatteryError>;

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
/// Errors that can occur while processing ACPI battery requests.
pub enum AcpiBatteryError {
    /// The provided battery ID does not correspond to any known battery device.
    UnknownDeviceId = 1,

    /// An unspecified error occurred while processing the request.
    UnspecifiedFailure = 2,
}

impl SerializableMessage for AcpiBatteryError {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            AcpiBatteryError::UnknownDeviceId | AcpiBatteryError::UnspecifiedFailure => Ok(0),
        }
    }

    fn deserialize(discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        AcpiBatteryError::try_from(discriminant)
            .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))
    }

    fn discriminant(&self) -> u16 {
        (*self).into()
    }
}

impl From<BatteryError> for AcpiBatteryError {
    fn from(error: BatteryError) -> Self {
        match error {
            BatteryError::UnknownDeviceId => AcpiBatteryError::UnknownDeviceId,
            BatteryError::UnspecifiedFailure => AcpiBatteryError::UnspecifiedFailure,
        }
    }
}

fn safe_get_u8(buffer: &[u8], index: usize) -> Result<u8, MessageSerializationError> {
    buffer
        .get(index)
        .copied()
        .ok_or(MessageSerializationError::BufferTooSmall)
}

fn safe_get_dword(buffer: &[u8], index: usize) -> Result<u32, MessageSerializationError> {
    let bytes = buffer
        .get(index..index + 4)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)?;
    Ok(u32::from_le_bytes(bytes))
}

fn safe_get_bytes<const N: usize>(buffer: &[u8], index: usize) -> Result<[u8; N], MessageSerializationError> {
    buffer
        .get(index..index + N)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .try_into()
        .map_err(|_| MessageSerializationError::BufferTooSmall)
}

fn safe_put_u8(buffer: &mut [u8], index: usize, val: u8) -> Result<usize, MessageSerializationError> {
    *buffer.get_mut(index).ok_or(MessageSerializationError::BufferTooSmall)? = val;
    Ok(1)
}

fn safe_put_dword(buffer: &mut [u8], index: usize, val: u32) -> Result<usize, MessageSerializationError> {
    buffer
        .get_mut(index..index + 4)
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .copy_from_slice(&val.to_le_bytes());
    Ok(4)
}

fn safe_put_bytes(buffer: &mut [u8], index: usize, bytes: &[u8]) -> Result<usize, MessageSerializationError> {
    buffer
        .get_mut(index..index + bytes.len())
        .ok_or(MessageSerializationError::BufferTooSmall)?
        .copy_from_slice(bytes);
    Ok(bytes.len())
}

const BIX_MODEL_NUM_START_IDX: usize = 64;
const BIX_MODEL_NUM_END_IDX: usize = BIX_MODEL_NUM_START_IDX + STD_BIX_MODEL_SIZE;
const BIX_SERIAL_NUM_START_IDX: usize = BIX_MODEL_NUM_END_IDX;
const BIX_SERIAL_NUM_END_IDX: usize = BIX_SERIAL_NUM_START_IDX + STD_BIX_SERIAL_SIZE;
const BIX_BATTERY_TYPE_START_IDX: usize = BIX_SERIAL_NUM_END_IDX;
const BIX_BATTERY_TYPE_END_IDX: usize = BIX_BATTERY_TYPE_START_IDX + STD_BIX_BATTERY_SIZE;
const BIX_OEM_INFO_START_IDX: usize = BIX_BATTERY_TYPE_END_IDX;
const BIX_OEM_INFO_END_IDX: usize = BIX_OEM_INFO_START_IDX + STD_BIX_OEM_SIZE;

fn bix_to_bytes(bix: BixFixedStrings, dst_slice: &mut [u8]) -> Result<usize, MessageSerializationError> {
    if dst_slice.len() < BIX_OEM_INFO_END_IDX + core::mem::size_of::<u32>() {
        return Err(MessageSerializationError::BufferTooSmall);
    }

    Ok(safe_put_dword(dst_slice, 0, bix.revision)?
        + safe_put_dword(dst_slice, 4, bix.power_unit.into())?
        + safe_put_dword(dst_slice, 8, bix.design_capacity)?
        + safe_put_dword(dst_slice, 12, bix.last_full_charge_capacity)?
        + safe_put_dword(dst_slice, 16, bix.battery_technology.into())?
        + safe_put_dword(dst_slice, 20, bix.design_voltage)?
        + safe_put_dword(dst_slice, 24, bix.design_cap_of_warning)?
        + safe_put_dword(dst_slice, 28, bix.design_cap_of_low)?
        + safe_put_dword(dst_slice, 32, bix.cycle_count)?
        + safe_put_dword(dst_slice, 36, bix.measurement_accuracy)?
        + safe_put_dword(dst_slice, 40, bix.max_sampling_time)?
        + safe_put_dword(dst_slice, 44, bix.min_sampling_time)?
        + safe_put_dword(dst_slice, 48, bix.max_averaging_interval)?
        + safe_put_dword(dst_slice, 52, bix.min_averaging_interval)?
        + safe_put_dword(dst_slice, 56, bix.battery_capacity_granularity_1)?
        + safe_put_dword(dst_slice, 60, bix.battery_capacity_granularity_2)?
        + safe_put_bytes(dst_slice, BIX_MODEL_NUM_START_IDX, &bix.model_number)?
        + safe_put_bytes(dst_slice, BIX_SERIAL_NUM_START_IDX, &bix.serial_number)?
        + safe_put_bytes(dst_slice, BIX_BATTERY_TYPE_START_IDX, &bix.battery_type)?
        + safe_put_bytes(dst_slice, BIX_OEM_INFO_START_IDX, &bix.oem_info)?
        + safe_put_dword(dst_slice, BIX_OEM_INFO_END_IDX, bix.battery_swapping_capability.into())?)
}

fn bix_from_bytes(src_slice: &[u8]) -> Result<BixFixedStrings, MessageSerializationError> {
    Ok(BixFixedStrings {
        revision: safe_get_dword(src_slice, 0)?,
        power_unit: safe_get_dword(src_slice, 4)?
            .try_into()
            .map_err(|_| MessageSerializationError::InvalidPayload("Invalid PowerUnit"))?,
        design_capacity: safe_get_dword(src_slice, 8)?,
        last_full_charge_capacity: safe_get_dword(src_slice, 12)?,
        battery_technology: safe_get_dword(src_slice, 16)?
            .try_into()
            .map_err(|_| MessageSerializationError::InvalidPayload("Invalid BatteryTechnology"))?,
        design_voltage: safe_get_dword(src_slice, 20)?,
        design_cap_of_warning: safe_get_dword(src_slice, 24)?,
        design_cap_of_low: safe_get_dword(src_slice, 28)?,
        cycle_count: safe_get_dword(src_slice, 32)?,
        measurement_accuracy: safe_get_dword(src_slice, 36)?,
        max_sampling_time: safe_get_dword(src_slice, 40)?,
        min_sampling_time: safe_get_dword(src_slice, 44)?,
        max_averaging_interval: safe_get_dword(src_slice, 48)?,
        min_averaging_interval: safe_get_dword(src_slice, 52)?,
        battery_capacity_granularity_1: safe_get_dword(src_slice, 56)?,
        battery_capacity_granularity_2: safe_get_dword(src_slice, 60)?,
        model_number: safe_get_bytes::<STD_BIX_MODEL_SIZE>(src_slice, BIX_MODEL_NUM_START_IDX)?,
        serial_number: safe_get_bytes::<STD_BIX_SERIAL_SIZE>(src_slice, BIX_SERIAL_NUM_START_IDX)?,
        battery_type: safe_get_bytes::<STD_BIX_BATTERY_SIZE>(src_slice, BIX_BATTERY_TYPE_START_IDX)?,
        oem_info: safe_get_bytes::<STD_BIX_OEM_SIZE>(src_slice, BIX_OEM_INFO_START_IDX)?,
        battery_swapping_capability: safe_get_dword(src_slice, BIX_OEM_INFO_END_IDX)?
            .try_into()
            .map_err(|_| MessageSerializationError::InvalidPayload("Invalid BatterySwappingCapability"))?,
    })
}

const PIF_MODEL_NUM_START_IDX: usize = 12;
const PIF_MODEL_NUM_END_IDX: usize = PIF_MODEL_NUM_START_IDX + STD_PIF_MODEL_SIZE;
const PIF_SERIAL_NUM_START_IDX: usize = PIF_MODEL_NUM_END_IDX;
const PIF_SERIAL_NUM_END_IDX: usize = PIF_SERIAL_NUM_START_IDX + STD_PIF_SERIAL_SIZE;
const PIF_OEM_INFO_START_IDX: usize = PIF_SERIAL_NUM_END_IDX;
const PIF_OEM_INFO_END_IDX: usize = PIF_OEM_INFO_START_IDX + STD_PIF_OEM_SIZE;

fn pif_to_bytes(pif: PifFixedStrings, dst_slice: &mut [u8]) -> Result<usize, MessageSerializationError> {
    if dst_slice.len() < PIF_OEM_INFO_END_IDX {
        return Err(MessageSerializationError::BufferTooSmall);
    }

    Ok(safe_put_dword(dst_slice, 0, pif.power_source_state.bits())?
        + safe_put_dword(dst_slice, 4, pif.max_output_power)?
        + safe_put_dword(dst_slice, 8, pif.max_input_power)?
        + safe_put_bytes(dst_slice, PIF_MODEL_NUM_START_IDX, &pif.model_number)?
        + safe_put_bytes(dst_slice, PIF_SERIAL_NUM_START_IDX, &pif.serial_number)?
        + safe_put_bytes(dst_slice, PIF_OEM_INFO_START_IDX, &pif.oem_info)?)
}

fn pif_from_bytes(src_slice: &[u8]) -> Result<PifFixedStrings, MessageSerializationError> {
    Ok(PifFixedStrings {
        power_source_state: PowerSourceState::from_bits(safe_get_dword(src_slice, 0)?)
            .ok_or(MessageSerializationError::InvalidPayload("Invalid PowerSourceState"))?,
        max_output_power: safe_get_dword(src_slice, 4)?,
        max_input_power: safe_get_dword(src_slice, 8)?,
        model_number: safe_get_bytes::<STD_PIF_MODEL_SIZE>(src_slice, PIF_MODEL_NUM_START_IDX)?,
        serial_number: safe_get_bytes::<STD_PIF_SERIAL_SIZE>(src_slice, PIF_SERIAL_NUM_START_IDX)?,
        oem_info: safe_get_bytes::<STD_PIF_OEM_SIZE>(src_slice, PIF_OEM_INFO_START_IDX)?,
    })
}
