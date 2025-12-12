#![no_std]

use embedded_batteries_async::acpi::{
    BCT_RETURN_SIZE_BYTES, BMD_RETURN_SIZE_BYTES, BPC_RETURN_SIZE_BYTES, BPS_RETURN_SIZE_BYTES, BST_RETURN_SIZE_BYTES,
    BTM_RETURN_SIZE_BYTES, PSR_RETURN_SIZE_BYTES, STA_RETURN_SIZE_BYTES,
};
use embedded_services::ec_type::message::{MessageSerializationError, SerializableMessage};

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

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BixFixedStrings {
    /// Revision of the BIX structure. Current revision is 1.
    pub revision: u32,
    /// Unit used for capacity and rate values.
    pub power_unit: embedded_batteries_async::acpi::PowerUnit,
    /// Design capacity of the battery (in mWh or mAh).
    pub design_capacity: u32,
    /// Last full charge capacity (in mWh or mAh).
    pub last_full_charge_capacity: u32,
    /// Battery technology type.
    pub battery_technology: embedded_batteries_async::acpi::BatteryTechnology,
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
    pub battery_swapping_capability: embedded_batteries_async::acpi::BatterySwapCapability,
}

// TODO this is essentially a hand-written reinterpret_cast - can we codegen some of this instead?
impl BixFixedStrings {
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<(), MessageSerializationError> {
        const MODEL_NUM_START_IDX: usize = 64;
        let model_num_end_idx: usize = MODEL_NUM_START_IDX + STD_BIX_MODEL_SIZE;
        let serial_num_start_idx = model_num_end_idx;
        let serial_num_end_idx = serial_num_start_idx + STD_BIX_SERIAL_SIZE;
        let battery_type_start_idx = serial_num_end_idx;
        let battery_type_end_idx = battery_type_start_idx + STD_BIX_BATTERY_SIZE;
        let oem_info_start_idx = battery_type_end_idx;
        let oem_info_end_idx = oem_info_start_idx + STD_BIX_OEM_SIZE;

        if dst_slice.len() < oem_info_end_idx {
            return Err(MessageSerializationError::BufferTooSmall);
        }

        dst_slice
            .get_mut(..4)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.revision));
        dst_slice
            .get_mut(4..8)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.power_unit.into()));
        dst_slice
            .get_mut(8..12)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.design_capacity));
        dst_slice
            .get_mut(12..16)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.last_full_charge_capacity));
        dst_slice
            .get_mut(16..20)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.battery_technology.into()));
        dst_slice
            .get_mut(20..24)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.design_voltage));
        dst_slice
            .get_mut(24..28)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.design_cap_of_warning));
        dst_slice
            .get_mut(28..32)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.design_cap_of_low));
        dst_slice
            .get_mut(32..36)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.cycle_count));
        dst_slice
            .get_mut(36..40)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.measurement_accuracy));
        dst_slice
            .get_mut(40..44)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.max_sampling_time));
        dst_slice
            .get_mut(44..48)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.min_sampling_time));
        dst_slice
            .get_mut(48..52)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.max_averaging_interval));
        dst_slice
            .get_mut(52..56)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.min_averaging_interval));
        dst_slice
            .get_mut(56..60)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.battery_capacity_granularity_1));
        dst_slice
            .get_mut(60..64)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.battery_capacity_granularity_2));
        dst_slice
            .get_mut(MODEL_NUM_START_IDX..model_num_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.model_number);
        dst_slice
            .get_mut(serial_num_start_idx..serial_num_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.serial_number);
        dst_slice
            .get_mut(battery_type_start_idx..battery_type_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.battery_type);
        dst_slice
            .get_mut(oem_info_start_idx..oem_info_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.oem_info);
        dst_slice
            .get_mut(oem_info_end_idx..oem_info_end_idx + 4)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.battery_swapping_capability.into()));
        Ok(())
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PifFixedStrings {
    /// Bitfield describing the state and characteristics of the power source.
    pub power_source_state: embedded_batteries_async::acpi::PowerSourceState,
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
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<(), MessageSerializationError> {
        const MODEL_NUM_START_IDX: usize = 12;
        let model_num_end_idx: usize = MODEL_NUM_START_IDX + STD_BIX_MODEL_SIZE;
        let serial_num_start_idx = model_num_end_idx;
        let serial_num_end_idx = serial_num_start_idx + STD_BIX_SERIAL_SIZE;
        let oem_info_start_idx = serial_num_end_idx;
        let oem_info_end_idx = oem_info_start_idx + STD_BIX_OEM_SIZE;

        if dst_slice.len() < oem_info_end_idx {
            return Err(MessageSerializationError::BufferTooSmall);
        }

        dst_slice
            .get_mut(..4)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.power_source_state.bits()));
        dst_slice
            .get_mut(4..8)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.max_output_power));
        dst_slice
            .get_mut(8..12)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&u32::to_le_bytes(self.max_input_power));
        dst_slice
            .get_mut(MODEL_NUM_START_IDX..model_num_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.model_number);
        dst_slice
            .get_mut(serial_num_start_idx..serial_num_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.serial_number);
        dst_slice
            .get_mut(oem_info_start_idx..oem_info_end_idx)
            .ok_or(MessageSerializationError::BufferTooSmall)?
            .copy_from_slice(&self.oem_info);
        Ok(())
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AcpiBatteryRequest {
    // TODO @Matteo/@Kurtis it looks like the ACPI spec doesn't support having a single device ID for multiple batteries - would it make sense to remove battery_id from each of these and say that anyone who wants to have multiple batteries should just instantiate a second instance of the battery service on a different endpoint?
    BatteryGetBixRequest {
        battery_id: u8,
    },
    BatteryGetBstRequest {
        battery_id: u8,
    },
    BatteryGetPsrRequest {
        battery_id: u8,
    },
    BatteryGetPifRequest {
        battery_id: u8,
    },
    BatteryGetBpsRequest {
        battery_id: u8,
    },
    BatterySetBtpRequest {
        battery_id: u8,
        btp: embedded_batteries_async::acpi::Btp,
    },
    BatterySetBptRequest {
        battery_id: u8,
        bpt: embedded_batteries_async::acpi::Bpt,
    },
    BatteryGetBpcRequest {
        battery_id: u8,
    },
    BatterySetBmcRequest {
        battery_id: u8,
        bmc: embedded_batteries_async::acpi::Bmc,
    },
    BatteryGetBmdRequest {
        battery_id: u8,
    },
    BatteryGetBctRequest {
        battery_id: u8,
        bct: embedded_batteries_async::acpi::Bct,
    },
    BatteryGetBtmRequest {
        battery_id: u8,
        btm: embedded_batteries_async::acpi::Btm,
    },
    BatterySetBmsRequest {
        battery_id: u8,
        bms: embedded_batteries_async::acpi::Bms,
    },
    BatterySetBmaRequest {
        battery_id: u8,
        bma: embedded_batteries_async::acpi::Bma,
    },
    BatteryGetStaRequest {
        battery_id: u8,
    },
}

impl SerializableMessage for AcpiBatteryRequest {
    fn serialize(self, _buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to serialize requests on the EC side",
        ))
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
                    btp: embedded_batteries_async::acpi::Btp {
                        trip_point: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBpt => Self::BatterySetBptRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bpt: embedded_batteries_async::acpi::Bpt {
                        revision: safe_get_dword(buffer, 1)?,
                        threshold_id: match safe_get_dword(buffer, 5)? {
                            0 => embedded_batteries_async::acpi::ThresholdId::ClearAll,
                            1 => embedded_batteries_async::acpi::ThresholdId::InstantaneousPeakPower,
                            2 => embedded_batteries_async::acpi::ThresholdId::SustainablePeakPower,
                            _ => {
                                return Err(MessageSerializationError::InvalidPayload("Unsupported threshold id"));
                            }
                        },
                        threshold_value: safe_get_dword(buffer, 9)?,
                    },
                },
                BatteryCmd::GetBpc => Self::BatteryGetBpcRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::SetBmc => Self::BatterySetBmcRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bmc: embedded_batteries_async::acpi::Bmc {
                        maintenance_control_flags: embedded_batteries_async::acpi::BmcControlFlags::from_bits_retain(
                            safe_get_dword(buffer, 1)?,
                        ),
                    },
                },
                BatteryCmd::GetBmd => Self::BatteryGetBmdRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                },
                BatteryCmd::GetBct => Self::BatteryGetBctRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bct: embedded_batteries_async::acpi::Bct {
                        charge_level_percent: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::GetBtm => Self::BatteryGetBtmRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    btm: embedded_batteries_async::acpi::Btm {
                        discharge_rate: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBms => Self::BatterySetBmsRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bms: embedded_batteries_async::acpi::Bms {
                        sampling_time_ms: safe_get_dword(buffer, 1)?,
                    },
                },
                BatteryCmd::SetBma => Self::BatterySetBmaRequest {
                    battery_id: safe_get_u8(buffer, 0)?,
                    bma: embedded_batteries_async::acpi::Bma {
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
    BatteryGetBixResponse {
        bix: BixFixedStrings,
    },
    BatteryGetBstResponse {
        bst: embedded_batteries_async::acpi::BstReturn,
    },
    BatteryGetPsrResponse {
        psr: embedded_batteries_async::acpi::PsrReturn,
    },
    BatteryGetPifResponse {
        pif: PifFixedStrings,
    },
    BatteryGetBpsResponse {
        bps: embedded_batteries_async::acpi::Bps,
    },
    BatterySetBtpResponse {},
    BatterySetBptResponse {},
    BatteryGetBpcResponse {
        bpc: embedded_batteries_async::acpi::Bpc,
    },
    BatterySetBmcResponse {},
    BatteryGetBmdResponse {
        bmd: embedded_batteries_async::acpi::Bmd,
    },
    BatteryGetBctResponse {
        bct_response: embedded_batteries_async::acpi::BctReturnResult,
    },
    BatteryGetBtmResponse {
        btm_response: embedded_batteries_async::acpi::BtmReturnResult,
    },
    BatterySetBmsResponse {
        status: u32,
    },
    BatterySetBmaResponse {
        status: u32,
    },
    BatteryGetStaResponse {
        sta: embedded_batteries_async::acpi::StaReturn,
    },
}

impl SerializableMessage for AcpiBatteryResponse {
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Self::BatteryGetBixResponse { bix } => bix.to_bytes(buffer).map(|_| 100),
            Self::BatteryGetBstResponse { bst } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bst.battery_state.bits()));
                buffer
                    .get_mut(4..8)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bst.battery_present_rate));
                buffer
                    .get_mut(8..12)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bst.battery_remaining_capacity));
                buffer
                    .get_mut(12..16)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bst.battery_present_voltage));

                Ok(BST_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetPsrResponse { psr } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(psr.power_source.into()));

                Ok(PSR_RETURN_SIZE_BYTES)
            }

            Self::BatteryGetPifResponse { pif } => pif.to_bytes(buffer).map(|_| 36),
            Self::BatteryGetBpsResponse { bps } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bps.revision));
                buffer
                    .get_mut(4..8)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bps.instantaneous_peak_power_level));
                buffer
                    .get_mut(8..12)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bps.instantaneous_peak_power_period));
                buffer
                    .get_mut(12..16)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bps.sustainable_peak_power_level));
                buffer
                    .get_mut(16..20)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bps.sustainable_peak_power_period));

                Ok(BPS_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBtpResponse {} => Ok(0),
            Self::BatterySetBptResponse {} => Ok(0),
            Self::BatteryGetBpcResponse { bpc } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bpc.revision));
                buffer
                    .get_mut(4..8)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bpc.power_threshold_support.bits()));
                buffer
                    .get_mut(8..12)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bpc.max_instantaneous_peak_power_threshold));
                buffer
                    .get_mut(12..16)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bpc.max_sustainable_peak_power_threshold));

                Ok(BPC_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBmcResponse {} => Ok(0),
            Self::BatteryGetBmdResponse { bmd } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bmd.status_flags.bits()));
                buffer
                    .get_mut(4..8)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bmd.capability_flags.bits()));
                buffer
                    .get_mut(8..12)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bmd.recalibrate_count));
                buffer
                    .get_mut(12..16)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bmd.quick_recalibrate_time));
                buffer
                    .get_mut(16..20)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bmd.slow_recalibrate_time));

                Ok(BMD_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetBctResponse { bct_response } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(bct_response.into()));

                Ok(BCT_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetBtmResponse { btm_response } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(btm_response.into()));

                Ok(BTM_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBmsResponse { status } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::BatterySetBmaResponse { status } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::BatteryGetStaResponse { sta } => {
                buffer
                    .get_mut(..4)
                    .ok_or(MessageSerializationError::BufferTooSmall)?
                    .copy_from_slice(&u32::to_le_bytes(sta.bits()));

                Ok(STA_RETURN_SIZE_BYTES)
            }
        }
    }

    fn deserialize(_discriminant: u16, _buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        Err(MessageSerializationError::Other(
            "unimplemented - don't need to deserialize responses on the EC side",
        ))
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
