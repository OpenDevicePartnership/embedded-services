#![no_std]

use embedded_batteries_async::acpi::ThresholdId;
pub use embedded_batteries_async::acpi::{
    BatteryState, BatterySwapCapability, BatteryTechnology, Bct, BctReturnResult, Bma, Bmc, BmcControlFlags, Bmd,
    BmdCapabilityFlags, BmdStatusFlags, Bms, Bpc, Bps, Bpt, BstReturn, Btm, BtmReturnResult, Btp, PowerSource,
    PowerSourceState, PowerThresholdSupport, PowerUnit, PsrReturn, StaReturn,
};
use embedded_services::relay::{MessageSerializationError, SerializableMessage};

// Unfortunately `TryFrom<u32>` is not implemented by embedded-batteries for these types

/// Attempt to convert a `u32` to a `PowerUnit`.
pub fn power_unit_try_from_u32(value: u32) -> Result<PowerUnit, MessageSerializationError> {
    match value {
        0 => Ok(PowerUnit::MilliWatts),
        1 => Ok(PowerUnit::MilliAmps),
        _ => Err(MessageSerializationError::InvalidPayload("Invalid PowerUnit")),
    }
}

/// Attempt to convert a `u32` to a `BatteryTechnology`.
pub fn bat_tech_try_from_u32(value: u32) -> Result<BatteryTechnology, MessageSerializationError> {
    match value {
        0 => Ok(BatteryTechnology::Primary),
        1 => Ok(BatteryTechnology::Secondary),
        _ => Err(MessageSerializationError::InvalidPayload("Invalid BatteryTechnology")),
    }
}

/// Attempt to convert a `u32` to a `BatterySwapCapability`.
pub fn bat_swap_try_from_u32(value: u32) -> Result<BatterySwapCapability, MessageSerializationError> {
    match value {
        0 => Ok(BatterySwapCapability::NonSwappable),
        1 => Ok(BatterySwapCapability::ColdSwappable),
        2 => Ok(BatterySwapCapability::HotSwappable),
        _ => Err(MessageSerializationError::InvalidPayload("Invalid BatteryTechnology")),
    }
}

/// Attempt to convert a `u32` to a `ThresholdId`.
pub fn thres_id_try_from_u32(value: u32) -> Result<ThresholdId, MessageSerializationError> {
    match value {
        0 => Ok(ThresholdId::ClearAll),
        1 => Ok(ThresholdId::InstantaneousPeakPower),
        2 => Ok(ThresholdId::SustainablePeakPower),
        _ => Err(MessageSerializationError::InvalidPayload("Invalid ThresholdId")),
    }
}

/// Attempt to convert a `u32` to a `PowerSource`.
pub fn pwr_src_try_from_u32(value: u32) -> Result<PowerSource, MessageSerializationError> {
    match value {
        0 => Ok(PowerSource::Offline),
        1 => Ok(PowerSource::Online),
        _ => Err(MessageSerializationError::InvalidPayload("Invalid PowerSource")),
    }
}

/// Standard Battery Service Model Number String Size
pub const STD_BIX_MODEL_SIZE: usize = 8;
/// Standard Battery Service Serial Number String Size
pub const STD_BIX_SERIAL_SIZE: usize = 8;
/// Standard Battery Service Battery Type String Size
pub const STD_BIX_BATTERY_SIZE: usize = 8;
/// Standard Battery Service OEM Info String Size
pub const STD_BIX_OEM_SIZE: usize = 8;
/// Standard Power Policy Service Model Number String Size
pub const STD_PIF_MODEL_SIZE: usize = 8;
/// Standard Power Policy Serial Number String Size
pub const STD_PIF_SERIAL_SIZE: usize = 8;
/// Standard Power Policy Service OEM Info String Size
pub const STD_PIF_OEM_SIZE: usize = 8;

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
            AcpiBatteryRequest::BatteryGetBixRequest { .. } => BatteryCmd::GetBix,
            AcpiBatteryRequest::BatteryGetBstRequest { .. } => BatteryCmd::GetBst,
            AcpiBatteryRequest::BatteryGetPsrRequest { .. } => BatteryCmd::GetPsr,
            AcpiBatteryRequest::BatteryGetPifRequest { .. } => BatteryCmd::GetPif,
            AcpiBatteryRequest::BatteryGetBpsRequest { .. } => BatteryCmd::GetBps,
            AcpiBatteryRequest::BatterySetBtpRequest { .. } => BatteryCmd::SetBtp,
            AcpiBatteryRequest::BatterySetBptRequest { .. } => BatteryCmd::SetBpt,
            AcpiBatteryRequest::BatteryGetBpcRequest { .. } => BatteryCmd::GetBpc,
            AcpiBatteryRequest::BatterySetBmcRequest { .. } => BatteryCmd::SetBmc,
            AcpiBatteryRequest::BatteryGetBmdRequest { .. } => BatteryCmd::GetBmd,
            AcpiBatteryRequest::BatteryGetBctRequest { .. } => BatteryCmd::GetBct,
            AcpiBatteryRequest::BatteryGetBtmRequest { .. } => BatteryCmd::GetBtm,
            AcpiBatteryRequest::BatterySetBmsRequest { .. } => BatteryCmd::SetBms,
            AcpiBatteryRequest::BatterySetBmaRequest { .. } => BatteryCmd::SetBma,
            AcpiBatteryRequest::BatteryGetStaRequest { .. } => BatteryCmd::GetSta,
        }
    }
}

impl From<&AcpiBatteryResponse> for BatteryCmd {
    fn from(response: &AcpiBatteryResponse) -> Self {
        match response {
            AcpiBatteryResponse::BatteryGetBixResponse { .. } => BatteryCmd::GetBix,
            AcpiBatteryResponse::BatteryGetBstResponse { .. } => BatteryCmd::GetBst,
            AcpiBatteryResponse::BatteryGetPsrResponse { .. } => BatteryCmd::GetPsr,
            AcpiBatteryResponse::BatteryGetPifResponse { .. } => BatteryCmd::GetPif,
            AcpiBatteryResponse::BatteryGetBpsResponse { .. } => BatteryCmd::GetBps,
            AcpiBatteryResponse::BatterySetBtpResponse { .. } => BatteryCmd::SetBtp,
            AcpiBatteryResponse::BatterySetBptResponse { .. } => BatteryCmd::SetBpt,
            AcpiBatteryResponse::BatteryGetBpcResponse { .. } => BatteryCmd::GetBpc,
            AcpiBatteryResponse::BatterySetBmcResponse { .. } => BatteryCmd::SetBmc,
            AcpiBatteryResponse::BatteryGetBmdResponse { .. } => BatteryCmd::GetBmd,
            AcpiBatteryResponse::BatteryGetBctResponse { .. } => BatteryCmd::GetBct,
            AcpiBatteryResponse::BatteryGetBtmResponse { .. } => BatteryCmd::GetBtm,
            AcpiBatteryResponse::BatterySetBmsResponse { .. } => BatteryCmd::SetBms,
            AcpiBatteryResponse::BatterySetBmaResponse { .. } => BatteryCmd::SetBma,
            AcpiBatteryResponse::BatteryGetStaResponse { .. } => BatteryCmd::GetSta,
        }
    }
}

const BIX_MODEL_NUM_START_IDX: usize = 64;
const BIX_MODEL_NUM_END_IDX: usize = BIX_MODEL_NUM_START_IDX + STD_BIX_MODEL_SIZE;
const BIX_SERIAL_NUM_START_IDX: usize = BIX_MODEL_NUM_END_IDX;
const BIX_SERIAL_NUM_END_IDX: usize = BIX_SERIAL_NUM_START_IDX + STD_BIX_SERIAL_SIZE;
const BIX_BATTERY_TYPE_START_IDX: usize = BIX_SERIAL_NUM_END_IDX;
const BIX_BATTERY_TYPE_END_IDX: usize = BIX_BATTERY_TYPE_START_IDX + STD_BIX_BATTERY_SIZE;
const BIX_OEM_INFO_START_IDX: usize = BIX_BATTERY_TYPE_END_IDX;
const BIX_OEM_INFO_END_IDX: usize = BIX_OEM_INFO_START_IDX + STD_BIX_OEM_SIZE;

#[derive(PartialEq, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BixFixedStrings {
    /// Revision of the BIX structure. Current revision is 1.
    pub revision: u32,
    /// Unit used for capacity and rate values.
    pub power_unit: PowerUnit,
    /// Design capacity of the battery (in mWh or mAh).
    pub design_capacity: u32,
    /// Last full charge capacity (in mWh or mAh).
    pub last_full_charge_capacity: u32,
    /// Battery technology type.
    pub battery_technology: BatteryTechnology,
    /// Design voltage (in mV).
    pub design_voltage: u32,
    /// Warning capacity threshold (in mWh or mAh).
    pub design_cap_of_warning: u32,
    /// Low capacity threshold (in mWh or mAh).
    pub design_cap_of_low: u32,
    /// Number of charge/discharge cycles.
    pub cycle_count: u32,
    /// Measurement accuracy in thousandths of a percent (e.g., 80000 = 80.000%).
    pub measurement_accuracy: u32,
    /// Maximum supported sampling time (in ms).
    pub max_sampling_time: u32,
    /// Minimum supported sampling time (in ms).
    pub min_sampling_time: u32,
    /// Maximum supported averaging interval (in ms).
    pub max_averaging_interval: u32,
    /// Minimum supported averaging interval (in ms).
    pub min_averaging_interval: u32,
    /// Capacity granularity between low and warning (in mWh or mAh).
    pub battery_capacity_granularity_1: u32,
    /// Capacity granularity between warning and full (in mWh or mAh).
    pub battery_capacity_granularity_2: u32,
    /// OEM-specific model number (ASCIIZ).
    pub model_number: [u8; STD_BIX_MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ).
    pub serial_number: [u8; STD_BIX_SERIAL_SIZE],
    /// OEM-specific battery type (ASCIIZ).
    pub battery_type: [u8; STD_BIX_BATTERY_SIZE],
    /// OEM-specific information (ASCIIZ).
    pub oem_info: [u8; STD_BIX_OEM_SIZE],
    /// Battery swapping capability.
    pub battery_swapping_capability: BatterySwapCapability,
}

// TODO this is essentially a hand-written reinterpret_cast - can we codegen some of this instead?
impl BixFixedStrings {
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<usize, MessageSerializationError> {
        if dst_slice.len() < BIX_OEM_INFO_END_IDX {
            return Err(MessageSerializationError::BufferTooSmall);
        }

        Ok(safe_put_dword(dst_slice, 0, self.revision)?
            + safe_put_dword(dst_slice, 4, self.power_unit.into())?
            + safe_put_dword(dst_slice, 8, self.design_capacity)?
            + safe_put_dword(dst_slice, 12, self.last_full_charge_capacity)?
            + safe_put_dword(dst_slice, 16, self.battery_technology.into())?
            + safe_put_dword(dst_slice, 20, self.design_voltage)?
            + safe_put_dword(dst_slice, 24, self.design_cap_of_warning)?
            + safe_put_dword(dst_slice, 28, self.design_cap_of_low)?
            + safe_put_dword(dst_slice, 32, self.cycle_count)?
            + safe_put_dword(dst_slice, 36, self.measurement_accuracy)?
            + safe_put_dword(dst_slice, 40, self.max_sampling_time)?
            + safe_put_dword(dst_slice, 44, self.min_sampling_time)?
            + safe_put_dword(dst_slice, 48, self.max_averaging_interval)?
            + safe_put_dword(dst_slice, 52, self.min_averaging_interval)?
            + safe_put_dword(dst_slice, 56, self.battery_capacity_granularity_1)?
            + safe_put_dword(dst_slice, 60, self.battery_capacity_granularity_2)?
            + safe_put_bytes(dst_slice, BIX_MODEL_NUM_START_IDX, &self.model_number)?
            + safe_put_bytes(dst_slice, BIX_SERIAL_NUM_START_IDX, &self.serial_number)?
            + safe_put_bytes(dst_slice, BIX_BATTERY_TYPE_START_IDX, &self.battery_type)?
            + safe_put_bytes(dst_slice, BIX_OEM_INFO_START_IDX, &self.oem_info)?
            + safe_put_dword(dst_slice, BIX_OEM_INFO_END_IDX, self.battery_swapping_capability.into())?)
    }

    pub fn from_bytes(src_slice: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(Self {
            revision: safe_get_dword(src_slice, 0)?,
            power_unit: power_unit_try_from_u32(safe_get_dword(src_slice, 4)?)?,
            design_capacity: safe_get_dword(src_slice, 8)?,
            last_full_charge_capacity: safe_get_dword(src_slice, 12)?,
            battery_technology: bat_tech_try_from_u32(safe_get_dword(src_slice, 16)?)?,
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
            battery_swapping_capability: bat_swap_try_from_u32(safe_get_dword(src_slice, BIX_OEM_INFO_END_IDX)?)?,
        })
    }
}

const PIF_MODEL_NUM_START_IDX: usize = 12;
const PIF_MODEL_NUM_END_IDX: usize = PIF_MODEL_NUM_START_IDX + STD_BIX_MODEL_SIZE;
const PIF_SERIAL_NUM_START_IDX: usize = PIF_MODEL_NUM_END_IDX;
const PIF_SERIAL_NUM_END_IDX: usize = PIF_SERIAL_NUM_START_IDX + STD_BIX_SERIAL_SIZE;
const PIF_OEM_INFO_START_IDX: usize = PIF_SERIAL_NUM_END_IDX;
const PIF_OEM_INFO_END_IDX: usize = PIF_OEM_INFO_START_IDX + STD_BIX_OEM_SIZE;

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PifFixedStrings {
    /// Bitfield describing the state and characteristics of the power source.
    pub power_source_state: PowerSourceState,
    /// Maximum rated output power in milliwatts (mW).
    ///
    /// 0xFFFFFFFF indicates the value is unavailable.
    pub max_output_power: u32,
    /// Maximum rated input power in milliwatts (mW).
    ///
    /// 0xFFFFFFFF indicates the value is unavailable.
    pub max_input_power: u32,
    /// OEM-specific model number (ASCIIZ). Empty string if not supported.
    pub model_number: [u8; STD_BIX_MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ). Empty string if not supported.
    pub serial_number: [u8; STD_BIX_SERIAL_SIZE],
    /// OEM-specific information (ASCIIZ). Empty string if not supported.
    pub oem_info: [u8; STD_BIX_OEM_SIZE],
}

impl PifFixedStrings {
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<usize, MessageSerializationError> {
        if dst_slice.len() < PIF_OEM_INFO_END_IDX {
            return Err(MessageSerializationError::BufferTooSmall);
        }

        Ok(safe_put_dword(dst_slice, 0, self.power_source_state.bits())?
            + safe_put_dword(dst_slice, 4, self.max_output_power)?
            + safe_put_dword(dst_slice, 8, self.max_input_power)?
            + safe_put_bytes(dst_slice, PIF_MODEL_NUM_START_IDX, &self.model_number)?
            + safe_put_bytes(dst_slice, PIF_SERIAL_NUM_START_IDX, &self.serial_number)?
            + safe_put_bytes(dst_slice, PIF_OEM_INFO_START_IDX, &self.oem_info)?)
    }

    pub fn from_bytes(src_slice: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(Self {
            power_source_state: PowerSourceState::from_bits(safe_get_dword(src_slice, 0)?)
                .ok_or(MessageSerializationError::InvalidPayload("Invalid PowerSourceState"))?,
            max_output_power: safe_get_dword(src_slice, 4)?,
            max_input_power: safe_get_dword(src_slice, 8)?,
            model_number: safe_get_bytes::<STD_BIX_MODEL_SIZE>(src_slice, PIF_MODEL_NUM_START_IDX)?,
            serial_number: safe_get_bytes::<STD_BIX_SERIAL_SIZE>(src_slice, PIF_SERIAL_NUM_START_IDX)?,
            oem_info: safe_get_bytes::<STD_BIX_OEM_SIZE>(src_slice, PIF_OEM_INFO_START_IDX)?,
        })
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AcpiBatteryRequest {
    BatteryGetBixRequest { battery_id: u8 },
    BatteryGetBstRequest { battery_id: u8 },
    BatteryGetPsrRequest { battery_id: u8 },
    BatteryGetPifRequest { battery_id: u8 },
    BatteryGetBpsRequest { battery_id: u8 },
    BatterySetBtpRequest { battery_id: u8, btp: Btp },
    BatterySetBptRequest { battery_id: u8, bpt: Bpt },
    BatteryGetBpcRequest { battery_id: u8 },
    BatterySetBmcRequest { battery_id: u8, bmc: Bmc },
    BatteryGetBmdRequest { battery_id: u8 },
    BatteryGetBctRequest { battery_id: u8, bct: Bct },
    BatteryGetBtmRequest { battery_id: u8, btm: Btm },
    BatterySetBmsRequest { battery_id: u8, bms: Bms },
    BatterySetBmaRequest { battery_id: u8, bma: Bma },
    BatteryGetStaRequest { battery_id: u8 },
}

impl SerializableMessage for AcpiBatteryRequest {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::BatteryGetBixRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatteryGetBstRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatteryGetPsrRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatteryGetPifRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatteryGetBpsRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatterySetBtpRequest { battery_id, btp } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, btp.trip_point)?)
            }
            Self::BatterySetBptRequest { battery_id, bpt } => Ok(safe_put_u8(buffer, 0, battery_id)?
                + safe_put_dword(buffer, 1, bpt.revision)?
                + safe_put_dword(buffer, 5, bpt.threshold_id as u32)?
                + safe_put_dword(buffer, 9, bpt.threshold_value)?),
            Self::BatteryGetBpcRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatterySetBmcRequest { battery_id, bmc } => {
                Ok(safe_put_u8(buffer, 0, battery_id)?
                    + safe_put_dword(buffer, 1, bmc.maintenance_control_flags.bits())?)
            }
            Self::BatteryGetBmdRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
            Self::BatteryGetBctRequest { battery_id, bct } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bct.charge_level_percent)?)
            }
            Self::BatteryGetBtmRequest { battery_id, btm } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, btm.discharge_rate)?)
            }
            Self::BatterySetBmsRequest { battery_id, bms } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bms.sampling_time_ms)?)
            }
            Self::BatterySetBmaRequest { battery_id, bma } => {
                Ok(safe_put_u8(buffer, 0, battery_id)? + safe_put_dword(buffer, 1, bma.averaging_interval_ms)?)
            }
            Self::BatteryGetStaRequest { battery_id } => safe_put_u8(buffer, 0, battery_id),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match BatteryCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                BatteryCmd::GetBix => Self::BatteryGetBixRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBst => Self::BatteryGetBstRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetPsr => Self::BatteryGetPsrRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetPif => Self::BatteryGetPifRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBps => Self::BatteryGetBpsRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::SetBtp => Self::BatterySetBtpRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    btp: Btp {
                        trip_point: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBpt => Self::BatterySetBptRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bpt: Bpt {
                        revision: safe_get_dword(buffer, 1)?,
                        threshold_id: thres_id_try_from_u32(safe_get_dword(buffer, 5)?)?,
                        threshold_value: safe_get_dword(buffer, 9)?,
                    },
                },
                BatteryCmd::GetBpc => Self::BatteryGetBpcRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::SetBmc => Self::BatterySetBmcRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bmc: Bmc {
                        maintenance_control_flags: BmcControlFlags::from_bits_retain(safe_get_dword(buffer, 1)?),
                    },
                },
                BatteryCmd::GetBmd => Self::BatteryGetBmdRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBct => Self::BatteryGetBctRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bct: Bct {
                        charge_level_percent: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::GetBtm => Self::BatteryGetBtmRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    btm: Btm {
                        discharge_rate: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBms => Self::BatterySetBmsRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bms: Bms {
                        sampling_time_ms: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBma => Self::BatterySetBmaRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bma: Bma {
                        averaging_interval_ms: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::GetSta => Self::BatteryGetStaRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
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
pub enum AcpiBatteryResponse {
    BatteryGetBixResponse { bix: BixFixedStrings },
    BatteryGetBstResponse { bst: BstReturn },
    BatteryGetPsrResponse { psr: PsrReturn },
    BatteryGetPifResponse { pif: PifFixedStrings },
    BatteryGetBpsResponse { bps: Bps },
    BatterySetBtpResponse {},
    BatterySetBptResponse {},
    BatteryGetBpcResponse { bpc: Bpc },
    BatterySetBmcResponse {},
    BatteryGetBmdResponse { bmd: Bmd },
    BatteryGetBctResponse { bct_response: BctReturnResult },
    BatteryGetBtmResponse { btm_response: BtmReturnResult },
    BatterySetBmsResponse { status: u32 },
    BatterySetBmaResponse { status: u32 },
    BatteryGetStaResponse { sta: StaReturn },
}

impl SerializableMessage for AcpiBatteryResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::BatteryGetBixResponse { bix } => bix.to_bytes(buffer),
            Self::BatteryGetBstResponse { bst } => Ok(safe_put_dword(buffer, 0, bst.battery_state.bits())?
                + safe_put_dword(buffer, 4, bst.battery_present_rate)?
                + safe_put_dword(buffer, 8, bst.battery_remaining_capacity)?
                + safe_put_dword(buffer, 12, bst.battery_present_voltage)?),
            Self::BatteryGetPsrResponse { psr } => safe_put_dword(buffer, 0, psr.power_source.into()),

            Self::BatteryGetPifResponse { pif } => pif.to_bytes(buffer),
            Self::BatteryGetBpsResponse { bps } => Ok(safe_put_dword(buffer, 0, bps.revision)?
                + safe_put_dword(buffer, 4, bps.instantaneous_peak_power_level)?
                + safe_put_dword(buffer, 8, bps.instantaneous_peak_power_period)?
                + safe_put_dword(buffer, 12, bps.sustainable_peak_power_level)?
                + safe_put_dword(buffer, 16, bps.sustainable_peak_power_period)?),
            Self::BatterySetBtpResponse {} => Ok(0),
            Self::BatterySetBptResponse {} => Ok(0),
            Self::BatteryGetBpcResponse { bpc } => Ok(safe_put_dword(buffer, 0, bpc.revision)?
                + safe_put_dword(buffer, 4, bpc.power_threshold_support.bits())?
                + safe_put_dword(buffer, 8, bpc.max_instantaneous_peak_power_threshold)?
                + safe_put_dword(buffer, 12, bpc.max_sustainable_peak_power_threshold)?),
            Self::BatterySetBmcResponse {} => Ok(0),
            Self::BatteryGetBmdResponse { bmd } => Ok(safe_put_dword(buffer, 0, bmd.status_flags.bits())?
                + safe_put_dword(buffer, 4, bmd.capability_flags.bits())?
                + safe_put_dword(buffer, 8, bmd.recalibrate_count)?
                + safe_put_dword(buffer, 12, bmd.quick_recalibrate_time)?
                + safe_put_dword(buffer, 16, bmd.slow_recalibrate_time)?),
            Self::BatteryGetBctResponse { bct_response } => safe_put_dword(buffer, 0, bct_response.into()),
            Self::BatteryGetBtmResponse { btm_response } => safe_put_dword(buffer, 0, btm_response.into()),
            Self::BatterySetBmsResponse { status } => safe_put_dword(buffer, 0, status),
            Self::BatterySetBmaResponse { status } => safe_put_dword(buffer, 0, status),
            Self::BatteryGetStaResponse { sta } => safe_put_dword(buffer, 0, sta.bits()),
        }
    }

    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Ok(
            match BatteryCmd::try_from(discriminant)
                .map_err(|_| MessageSerializationError::UnknownMessageDiscriminant(discriminant))?
            {
                BatteryCmd::GetBix => Self::BatteryGetBixResponse {
                    bix: BixFixedStrings::from_bytes(buffer)?,
                },
                BatteryCmd::GetBst => {
                    let bst = BstReturn {
                        battery_state: BatteryState::from_bits(safe_get_dword(buffer, 0)?)
                            .ok_or(MessageSerializationError::BufferTooSmall)?,
                        battery_present_rate: safe_get_dword(buffer, 4)?,
                        battery_remaining_capacity: safe_get_dword(buffer, 8)?,
                        battery_present_voltage: safe_get_dword(buffer, 12)?,
                    };
                    Self::BatteryGetBstResponse { bst }
                }
                BatteryCmd::GetPsr => Self::BatteryGetPsrResponse {
                    psr: PsrReturn {
                        power_source: pwr_src_try_from_u32(safe_get_dword(buffer, 0)?)?,
                    },
                },
                BatteryCmd::GetPif => Self::BatteryGetPifResponse {
                    pif: PifFixedStrings::from_bytes(buffer)?,
                },
                BatteryCmd::GetBps => Self::BatteryGetBpsResponse {
                    bps: Bps {
                        revision: safe_get_dword(buffer, 0)?,
                        instantaneous_peak_power_level: safe_get_dword(buffer, 4)?,
                        instantaneous_peak_power_period: safe_get_dword(buffer, 8)?,
                        sustainable_peak_power_level: safe_get_dword(buffer, 12)?,
                        sustainable_peak_power_period: safe_get_dword(buffer, 16)?,
                    },
                },
                BatteryCmd::SetBtp => Self::BatterySetBtpResponse {},
                BatteryCmd::SetBpt => Self::BatterySetBptResponse {},
                BatteryCmd::GetBpc => Self::BatteryGetBpcResponse {
                    bpc: Bpc {
                        revision: safe_get_dword(buffer, 0)?,
                        power_threshold_support: PowerThresholdSupport::from_bits(safe_get_dword(buffer, 4)?)
                            .ok_or(MessageSerializationError::InvalidPayload("Invalid BpcThresholdSupport"))?,
                        max_instantaneous_peak_power_threshold: safe_get_dword(buffer, 8)?,
                        max_sustainable_peak_power_threshold: safe_get_dword(buffer, 12)?,
                    },
                },
                BatteryCmd::SetBmc => Self::BatterySetBmcResponse {},
                BatteryCmd::GetBmd => Self::BatteryGetBmdResponse {
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
                BatteryCmd::GetBct => Self::BatteryGetBctResponse {
                    bct_response: safe_get_dword(buffer, 0)?.into(),
                },
                BatteryCmd::GetBtm => Self::BatteryGetBtmResponse {
                    btm_response: safe_get_dword(buffer, 0)?.into(),
                },
                BatteryCmd::SetBms => Self::BatterySetBmsResponse {
                    status: safe_get_dword(buffer, 0)?,
                },
                BatteryCmd::SetBma => Self::BatterySetBmaResponse {
                    status: safe_get_dword(buffer, 0)?,
                },
                BatteryCmd::GetSta => Self::BatteryGetStaResponse {
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

/// Fuel gauge ID
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum AcpiBatteryError {
    UnknownDeviceId = 1,
    UnspecifiedFailure = 2,
}

pub type AcpiBatteryResult = Result<AcpiBatteryResponse, AcpiBatteryError>;

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
