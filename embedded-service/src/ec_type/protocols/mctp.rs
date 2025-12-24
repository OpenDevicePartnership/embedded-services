#![allow(missing_docs)]

use core::ops::{Div, Mul};

use embedded_batteries_async::acpi::{
    BCT_RETURN_SIZE_BYTES, BMD_RETURN_SIZE_BYTES, BPC_RETURN_SIZE_BYTES, BPS_RETURN_SIZE_BYTES, BST_RETURN_SIZE_BYTES,
    BTM_RETURN_SIZE_BYTES, BatteryState, BmdCapabilityFlags, BmdStatusFlags, PSR_RETURN_SIZE_BYTES, PowerSourceState,
    PowerThresholdSupport, PsrReturn, STA_RETURN_SIZE_BYTES,
};

use mctp_rs::{
    MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, error::MctpPacketResult,
    mctp_completion_code::MctpCompletionCode,
};

/// Append an MCTP header to the front of a message.
/// Returns the message and its new total with the appended header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpError {
    /// Header is not at least 9 bytes long.
    InvalidHeaderSize,
    /// Wrong destination address.
    WrongDestinationAddr,
    /// Invalid command code.
    InvalidCommandCode,
    /// Invalid byte count, encoded byte count does not match MCTP message length.
    InvalidByteCount,
    /// Invalid header version. Should be 1.
    InvalidHeaderVersion,
    /// Invalid destination endpoint
    InvalidDestinationEndpoint,
    /// Invalid source endpoint.
    InvalidSourceEndpoint,
    /// Multi message not supported.
    InvalidFlags,
}

/// Data type for MCTP message underlying data size.
pub type PayloadLen = usize;

/// Max payload len, due to SMBUS block transaction limits.
pub const MAX_MCTP_BYTE_COUNT: usize = 69;
/// Payload len + bytes for destination target address, command code, and byte count.
pub const MAX_MCTP_PACKET_LEN: usize = MAX_MCTP_BYTE_COUNT + 3;

fn round_up_to_nearest_mod_4(unrounded: usize) -> usize {
    unrounded + (unrounded % 4)
}

/// Decode a header from and MCTP message.
/// Returns the underlying data and its service endpoint ID and the underlying data size.
pub fn handle_mctp_header(
    mctp_msg: &[u8],
    data: &mut [u8],
) -> Result<(crate::comms::EndpointID, PayloadLen), MctpError> {
    // assert we have at least 9 bytes, minimum
    if mctp_msg.len() < 9 {
        return Err(MctpError::InvalidHeaderSize);
    }

    // EC is at address 2, if we have anything other than 2 reject it.
    if mctp_msg[0] != 2 {
        return Err(MctpError::WrongDestinationAddr);
    }

    // MCTP command code is 0x0F.
    if mctp_msg[1] != 0x0F {
        return Err(MctpError::InvalidCommandCode);
    }

    // Check the byte count is correctly formed and is not larger than the max in the spec.
    if usize::from(mctp_msg[2]) > MAX_MCTP_BYTE_COUNT {
        return Err(MctpError::InvalidByteCount);
    }
    // Some eSPI controllers behave oddly if packet sizes aren't multiples of 4, so the MCTP message is padded
    // to multiples of 4.
    // Byte size + header size (3) + padding to align size to multiple of 4 should equal length of message.
    // Unfortunately since padding is variable, there is no way to validate byte count is truly correct.
    // There is a chance that the number of valid bytes exceeds the byte count if mctp_msg.len()
    // is not a multiple of 4 (and thus has padding bytes)
    if ((usize::from(mctp_msg[2]) + 3) + 3).div(4).mul(4) != mctp_msg.len() {
        return Err(MctpError::InvalidByteCount);
    }

    // Only support header version 1.
    if mctp_msg[4] != 1 {
        return Err(MctpError::InvalidHeaderVersion);
    }

    // Subsystems supported currently are battery (0x02), thermal (0x03), and debug (0x04).
    let endpoint_id = match mctp_msg[5] {
        2 => crate::comms::EndpointID::Internal(crate::comms::Internal::Battery),
        3 => crate::comms::EndpointID::Internal(crate::comms::Internal::Thermal),
        4 => crate::comms::EndpointID::Internal(crate::comms::Internal::Debug),
        _ => return Err(MctpError::InvalidDestinationEndpoint),
    };

    // Only source endpoint supported currently is host (1).
    if mctp_msg[6] != 1 {
        return Err(MctpError::InvalidSourceEndpoint);
    }

    let som = mctp_msg[7] & (1 << 7) != 0;
    let eom = mctp_msg[7] & (1 << 6) != 0;
    let seq_num = (mctp_msg[7] & 0b0011_0000) >> 4;
    let msg_tag = mctp_msg[7] & 0b0000_0111;

    // Verify flags
    if !som || !eom || seq_num != 1 || msg_tag != 3 {
        return Err(MctpError::InvalidFlags);
    }

    let len = usize::from(mctp_msg[2]) - 5;
    // Copy message contents without the padding to a multiple of 4 at the end.
    data[..len].copy_from_slice(&mctp_msg[8..8 + len]);

    Ok((endpoint_id, len))
}

/// Append an MCTP header to the front of a message.
/// Returns the message and its new total with the appended header.
pub fn build_mctp_header(
    data: &[u8],
    data_len: usize,
    src_endpoint: crate::comms::EndpointID,
    start_of_msg: bool,
    end_of_msg: bool,
) -> Result<([u8; MAX_MCTP_PACKET_LEN], usize), MctpError> {
    let mut ret = [0u8; MAX_MCTP_PACKET_LEN];
    let padding = [0u8; 3];

    // Host is at address 0.
    ret[0] = 0;

    // MCTP command code is 0x0F.
    ret[1] = 0x0F;

    // Size of the payload length + header size, without padding
    ret[2] = (data_len + 5) as u8;

    // Source is EC (upper 7 bits = 0x01 | hardcoded LSB of 0x01)
    ret[3] = 3;

    // Header version is 1
    ret[4] = 1;

    // Destination endpoint ID is Host (0x01)
    ret[5] = 1;

    // Subsystems supported currently are battery (0x02), thermal (0x03), and debug (0x04).
    match src_endpoint {
        crate::comms::EndpointID::Internal(crate::comms::Internal::Battery) => ret[6] = 2,
        crate::comms::EndpointID::Internal(crate::comms::Internal::Thermal) => ret[6] = 3,
        crate::comms::EndpointID::Internal(crate::comms::Internal::Debug) => ret[6] = 4,
        _ => return Err(MctpError::InvalidDestinationEndpoint),
    }

    // Seq num 1 + Msg tag 3
    ret[7] = 0x13;
    if start_of_msg {
        ret[7] |= 1 << 7;
    }
    if end_of_msg {
        ret[7] |= 1 << 6;
    }

    // True packet size must be a multple of 4. Header is 8 bytes which is already a multiple of 4,
    // so we don't need to include it here.
    let data_len_padded = round_up_to_nearest_mod_4(data_len);

    ret[8..data_len + 8].copy_from_slice(&data[..data_len]);

    // Add padding to align to 4 bytes
    ret[data_len + 8..data_len_padded + 8].copy_from_slice(&padding[..data_len_padded - data_len]);

    Ok((ret, data_len_padded + 8))
}

// 5 bits total
#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum OdpService {
    Battery = 0x01,
    Thermal = 0x02,
    Debug = 0x03,
}

// 10 bits total
// TODO: Fully define offsets for subsystem, temporarily it is every 32 entries
#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum OdpCommandCode {
    BatteryGetBixRequest = 0x01,
    BatteryGetBstRequest = 0x02,
    BatteryGetPsrRequest = 0x03,
    BatteryGetPifRequest = 0x04,
    BatteryGetBpsRequest = 0x05,
    BatterySetBtpRequest = 0x06,
    BatterySetBptRequest = 0x07,
    BatteryGetBpcRequest = 0x08,
    BatterySetBmcRequest = 0x09,
    BatteryGetBmdRequest = 0x0A,
    BatteryGetBctRequest = 0x0B,
    BatteryGetBtmRequest = 0x0C,
    BatterySetBmsRequest = 0x0D,
    BatterySetBmaRequest = 0x0E,
    BatteryGetStaRequest = 0x0F,
    BatteryGetBixResponse = 0x11,
    BatteryGetBstResponse = 0x12,
    BatteryGetPsrResponse = 0x13,
    BatteryGetPifResponse = 0x14,
    BatteryGetBpsResponse = 0x15,
    BatterySetBtpResponse = 0x16,
    BatterySetBptResponse = 0x17,
    BatteryGetBpcResponse = 0x18,
    BatterySetBmcResponse = 0x19,
    BatteryGetBmdResponse = 0x1A,
    BatteryGetBctResponse = 0x1B,
    BatteryGetBtmResponse = 0x1C,
    BatterySetBmsResponse = 0x1D,
    BatterySetBmaResponse = 0x1E,
    BatteryGetStaResponse = 0x1F,
    ThermalGetTmpRequest = 0x20,
    ThermalSetThrsRequest = 0x21,
    ThermalGetThrsRequest = 0x22,
    ThermalSetScpRequest = 0x23,
    ThermalGetVarRequest = 0x24,
    ThermalSetVarRequest = 0x25,
    ThermalGetTmpResponse = 0x30,
    ThermalSetThrsResponse = 0x31,
    ThermalGetThrsResponse = 0x32,
    ThermalSetScpResponse = 0x33,
    ThermalGetVarResponse = 0x34,
    ThermalSetVarResponse = 0x35,
    DebugGetMsgsRequest = 0x40,
    DebugGetMsgsResponse = 0x50,
}

// 3 byte header
#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OdpHeader {
    pub request_bit: bool,                   // [23:23] (1 bit)
    pub datagram_bit: bool,                  // [22:22] (1 bit)
    pub service: OdpService,                 // [18:21] (4 bits)
    pub command_code: OdpCommandCode,        // [8:17] (10 bits)
    pub completion_code: MctpCompletionCode, // [0:7] (8 bits)
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BixFixedStrings<
    const MODEL_SIZE: usize,
    const SERIAL_SIZE: usize,
    const BATTERY_SIZE: usize,
    const OEM_SIZE: usize,
> {
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
    pub model_number: [u8; MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ).
    pub serial_number: [u8; SERIAL_SIZE],
    /// OEM-specific battery type (ASCIIZ).
    pub battery_type: [u8; BATTERY_SIZE],
    /// OEM-specific information (ASCIIZ).
    pub oem_info: [u8; OEM_SIZE],
    /// Battery swapping capability.
    pub battery_swapping_capability: embedded_batteries_async::acpi::BatterySwapCapability,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Error type when serializing ODP return types with fixed size strings.
pub enum OdpSerializeErr {
    /// Input slice is too small to encapsulate all the fields.
    InputSliceTooSmall,
}

impl<const MODEL_SIZE: usize, const SERIAL_SIZE: usize, const BATTERY_SIZE: usize, const OEM_SIZE: usize>
    BixFixedStrings<MODEL_SIZE, SERIAL_SIZE, BATTERY_SIZE, OEM_SIZE>
{
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<(), OdpSerializeErr> {
        const MODEL_NUM_START_IDX: usize = 64;
        let model_num_end_idx: usize = MODEL_NUM_START_IDX + MODEL_SIZE;
        let serial_num_start_idx = model_num_end_idx;
        let serial_num_end_idx = serial_num_start_idx + SERIAL_SIZE;
        let battery_type_start_idx = serial_num_end_idx;
        let battery_type_end_idx = battery_type_start_idx + BATTERY_SIZE;
        let oem_info_start_idx = battery_type_end_idx;
        let oem_info_end_idx = oem_info_start_idx + OEM_SIZE;

        if dst_slice.len() < oem_info_end_idx {
            return Err(OdpSerializeErr::InputSliceTooSmall);
        }

        dst_slice[..4].copy_from_slice(&u32::to_le_bytes(self.revision));
        dst_slice[4..8].copy_from_slice(&u32::to_le_bytes(self.power_unit.into()));
        dst_slice[8..12].copy_from_slice(&u32::to_le_bytes(self.design_capacity));
        dst_slice[12..16].copy_from_slice(&u32::to_le_bytes(self.last_full_charge_capacity));
        dst_slice[16..20].copy_from_slice(&u32::to_le_bytes(self.battery_technology.into()));
        dst_slice[20..24].copy_from_slice(&u32::to_le_bytes(self.design_voltage));
        dst_slice[24..28].copy_from_slice(&u32::to_le_bytes(self.design_cap_of_warning));
        dst_slice[28..32].copy_from_slice(&u32::to_le_bytes(self.design_cap_of_low));
        dst_slice[32..36].copy_from_slice(&u32::to_le_bytes(self.cycle_count));
        dst_slice[36..40].copy_from_slice(&u32::to_le_bytes(self.measurement_accuracy));
        dst_slice[40..44].copy_from_slice(&u32::to_le_bytes(self.max_sampling_time));
        dst_slice[44..48].copy_from_slice(&u32::to_le_bytes(self.min_sampling_time));
        dst_slice[48..52].copy_from_slice(&u32::to_le_bytes(self.max_averaging_interval));
        dst_slice[52..56].copy_from_slice(&u32::to_le_bytes(self.min_averaging_interval));
        dst_slice[56..60].copy_from_slice(&u32::to_le_bytes(self.battery_capacity_granularity_1));
        dst_slice[60..64].copy_from_slice(&u32::to_le_bytes(self.battery_capacity_granularity_2));
        dst_slice[MODEL_NUM_START_IDX..model_num_end_idx].copy_from_slice(&self.model_number);
        dst_slice[serial_num_start_idx..serial_num_end_idx].copy_from_slice(&self.serial_number);
        dst_slice[battery_type_start_idx..battery_type_end_idx].copy_from_slice(&self.battery_type);
        dst_slice[oem_info_start_idx..oem_info_end_idx].copy_from_slice(&self.oem_info);
        dst_slice[oem_info_end_idx..oem_info_end_idx + 4]
            .copy_from_slice(&u32::to_le_bytes(self.battery_swapping_capability.into()));
        Ok(())
    }
}

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PifFixedStrings<const MODEL_SIZE: usize, const SERIAL_SIZE: usize, const OEM_SIZE: usize> {
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
    pub model_number: [u8; MODEL_SIZE],
    /// OEM-specific serial number (ASCIIZ). Empty string if not supported.
    pub serial_number: [u8; SERIAL_SIZE],
    /// OEM-specific information (ASCIIZ). Empty string if not supported.
    pub oem_info: [u8; OEM_SIZE],
}

impl<const MODEL_SIZE: usize, const SERIAL_SIZE: usize, const OEM_SIZE: usize>
    PifFixedStrings<MODEL_SIZE, SERIAL_SIZE, OEM_SIZE>
{
    pub fn to_bytes(self, dst_slice: &mut [u8]) -> Result<(), OdpSerializeErr> {
        const MODEL_NUM_START_IDX: usize = 12;
        let model_num_end_idx: usize = MODEL_NUM_START_IDX + MODEL_SIZE;
        let serial_num_start_idx = model_num_end_idx;
        let serial_num_end_idx = serial_num_start_idx + SERIAL_SIZE;
        let oem_info_start_idx = serial_num_end_idx;
        let oem_info_end_idx = oem_info_start_idx + OEM_SIZE;

        if dst_slice.len() < oem_info_end_idx {
            return Err(OdpSerializeErr::InputSliceTooSmall);
        }

        dst_slice[..4].copy_from_slice(&u32::to_le_bytes(self.power_source_state.bits()));
        dst_slice[4..8].copy_from_slice(&u32::to_le_bytes(self.max_output_power));
        dst_slice[8..12].copy_from_slice(&u32::to_le_bytes(self.max_input_power));
        dst_slice[MODEL_NUM_START_IDX..model_num_end_idx].copy_from_slice(&self.model_number);
        dst_slice[serial_num_start_idx..serial_num_end_idx].copy_from_slice(&self.serial_number);
        dst_slice[oem_info_start_idx..oem_info_end_idx].copy_from_slice(&self.oem_info);
        Ok(())
    }
}

/// Standard 32-bit DWORD
pub type Dword = u32;

/// 16-bit variable length
pub type VarLen = u16;

/// Instance ID
pub type InstanceId = u8;

/// Time in milliseconds
pub type Milliseconds = Dword;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = Dword;

#[derive(PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Odp<
    const BIX_MODEL_SIZE: usize,
    const BIX_SERIAL_SIZE: usize,
    const BIX_BATTERY_SIZE: usize,
    const BIX_OEM_SIZE: usize,
    const PIF_MODEL_SIZE: usize,
    const PIF_SERIAL_SIZE: usize,
    const PIF_OEM_SIZE: usize,
    const DEBUG_BUF_SIZE: usize,
> {
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
    BatteryGetBixResponse {
        bix: BixFixedStrings<BIX_MODEL_SIZE, BIX_SERIAL_SIZE, BIX_BATTERY_SIZE, BIX_OEM_SIZE>,
    },
    BatteryGetBstResponse {
        bst: embedded_batteries_async::acpi::BstReturn,
    },
    BatteryGetPsrResponse {
        psr: embedded_batteries_async::acpi::PsrReturn,
    },
    BatteryGetPifResponse {
        pif: PifFixedStrings<PIF_MODEL_SIZE, PIF_SERIAL_SIZE, PIF_OEM_SIZE>,
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
        status: Dword,
    },
    BatterySetBmaResponse {
        status: Dword,
    },
    BatteryGetStaResponse {
        sta: embedded_batteries_async::acpi::StaReturn,
    },

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
        policy_id: Dword,
        acoustic_lim: Dword,
        power_lim: Dword,
    },
    ThermalGetVarRequest {
        instance_id: u8,
        len: VarLen,
        var_uuid: uuid::Bytes,
    },
    ThermalSetVarRequest {
        instance_id: u8,
        len: VarLen,
        var_uuid: uuid::Bytes,
        set_var: Dword,
    },
    DebugGetMsgsRequest,

    ThermalGetTmpResponse {
        temperature: DeciKelvin,
    },
    ThermalSetThrsResponse {
        status: Dword,
    },
    ThermalGetThrsResponse {
        status: Dword,
        timeout: Milliseconds,
        low: DeciKelvin,
        high: DeciKelvin,
    },
    ThermalSetScpResponse {
        status: Dword,
    },
    ThermalGetVarResponse {
        status: Dword,
        val: Dword,
    },
    ThermalSetVarResponse {
        status: Dword,
    },
    DebugGetMsgsResponse {
        debug_buf: [u8; DEBUG_BUF_SIZE],
    },
    ErrorResponse {},
}

impl MctpMessageHeaderTrait for OdpHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        check_header_length(buffer)?;
        let command_code: u16 = self.command_code as u16;
        buffer[0] = (self.request_bit as u8) << 7
            | (self.datagram_bit as u8) << 6
            | ((self.service as u8) & 0b0000_1111) << 2
            | ((command_code >> 8) as u8 & 0b0000_0011);
        buffer[1] = (command_code & 0x00FF) as u8;
        buffer[2] = self.completion_code.into();
        Ok(3)
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        check_header_length(buffer)?;
        let request_bit = buffer[0] & 0b1000_0000 != 0;
        let datagram_bit = buffer[0] & 0b0100_0000 != 0;
        let service = (buffer[0] & 0b0011_1100) >> 2;
        let command_code = ((buffer[0] & 0b0000_0011) as u16) << 8 | (buffer[1] as u16);

        let completion_code = buffer[2]
            .try_into()
            .map_err(|_| MctpPacketError::HeaderParseError("invalid completion code"))?;
        let service = service
            .try_into()
            .map_err(|_| MctpPacketError::HeaderParseError("invalid odp service"))?;
        let command_code = command_code
            .try_into()
            .map_err(|_| MctpPacketError::HeaderParseError("invalid odp command code"))?;

        Ok((
            OdpHeader {
                request_bit,
                datagram_bit,
                service,
                command_code,
                completion_code,
            },
            &buffer[3..],
        ))
    }
}

impl<
    const BIX_MODEL_SIZE: usize,
    const BIX_SERIAL_SIZE: usize,
    const BIX_BATTERY_SIZE: usize,
    const BIX_OEM_SIZE: usize,
    const PIF_MODEL_SIZE: usize,
    const PIF_SERIAL_SIZE: usize,
    const PIF_OEM_SIZE: usize,
    const DEBUG_BUF_SIZE: usize,
> MctpMessageTrait<'_>
    for Odp<
        BIX_MODEL_SIZE,
        BIX_SERIAL_SIZE,
        BIX_BATTERY_SIZE,
        BIX_OEM_SIZE,
        PIF_MODEL_SIZE,
        PIF_SERIAL_SIZE,
        PIF_OEM_SIZE,
        DEBUG_BUF_SIZE,
    >
{
    const MESSAGE_TYPE: u8 = 0x7D;
    type Header = OdpHeader;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        match self {
            Self::BatteryGetBixRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatteryGetBstRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatteryGetPsrRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatteryGetPifRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatteryGetBpsRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatterySetBtpRequest { battery_id, btp } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(btp.trip_point));

                Ok(5)
            }
            Self::BatterySetBptRequest { battery_id, bpt } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(bpt.revision));
                buffer[5..9].copy_from_slice(&u32::to_le_bytes(match bpt.threshold_id {
                    embedded_batteries_async::acpi::ThresholdId::ClearAll => 0,
                    embedded_batteries_async::acpi::ThresholdId::InstantaneousPeakPower => 1,
                    embedded_batteries_async::acpi::ThresholdId::SustainablePeakPower => 2,
                }));
                buffer[9..13].copy_from_slice(&u32::to_le_bytes(bpt.threshold_value));

                Ok(13)
            }
            Self::BatteryGetBpcRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatterySetBmcRequest { battery_id, bmc } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(bmc.maintenance_control_flags.bits()));

                Ok(5)
            }
            Self::BatteryGetBmdRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::BatteryGetBctRequest { battery_id, bct } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(bct.charge_level_percent));

                Ok(5)
            }
            Self::BatteryGetBtmRequest { battery_id, btm } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(btm.discharge_rate));

                Ok(5)
            }
            Self::BatterySetBmsRequest { battery_id, bms } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(bms.sampling_time_ms));

                Ok(5)
            }
            Self::BatterySetBmaRequest { battery_id, bma } => {
                buffer[0] = battery_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(bma.averaging_interval_ms));

                Ok(5)
            }
            Self::BatteryGetStaRequest { battery_id } => write_to_buffer(buffer, [battery_id]),
            Self::ThermalGetTmpRequest { instance_id } => write_to_buffer(buffer, [instance_id]),
            Self::ThermalSetThrsRequest {
                instance_id,
                timeout,
                low,
                high,
            } => {
                buffer[0] = instance_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(timeout));
                buffer[5..9].copy_from_slice(&u32::to_le_bytes(low));
                buffer[9..13].copy_from_slice(&u32::to_le_bytes(high));

                Ok(13)
            }
            Self::ThermalGetThrsRequest { instance_id } => write_to_buffer(buffer, [instance_id]),
            Self::ThermalSetScpRequest {
                instance_id,
                policy_id,
                acoustic_lim,
                power_lim,
            } => {
                buffer[0] = instance_id;
                buffer[1..5].copy_from_slice(&u32::to_le_bytes(policy_id));
                buffer[5..9].copy_from_slice(&u32::to_le_bytes(acoustic_lim));
                buffer[9..13].copy_from_slice(&u32::to_le_bytes(power_lim));

                Ok(13)
            }
            Self::ThermalGetVarRequest {
                instance_id,
                len,
                var_uuid,
            } => {
                buffer[0] = instance_id;
                buffer[1..3].copy_from_slice(&u16::to_le_bytes(len));
                buffer[3..19].copy_from_slice(&var_uuid);

                Ok(19)
            }
            Self::ThermalSetVarRequest {
                instance_id,
                len,
                var_uuid,
                set_var,
            } => {
                buffer[0] = instance_id;
                buffer[1..3].copy_from_slice(&u16::to_le_bytes(len));
                buffer[3..19].copy_from_slice(&var_uuid);
                buffer[19..23].copy_from_slice(&u32::to_le_bytes(set_var));

                Ok(23)
            }
            Self::DebugGetMsgsRequest => Ok(0),
            Self::BatteryGetBixResponse { bix } => bix
                .to_bytes(buffer)
                .map(|_| 100)
                .map_err(|_| mctp_rs::MctpPacketError::HeaderParseError("Bix parse failed")),
            Self::BatteryGetBstResponse { bst } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(bst.battery_state.bits()));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(bst.battery_remaining_capacity));
                buffer[8..12].copy_from_slice(&u32::to_le_bytes(bst.battery_present_rate));
                buffer[12..16].copy_from_slice(&u32::to_le_bytes(bst.battery_present_voltage));

                Ok(BST_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetPsrResponse { psr } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(psr.power_source.into()));

                Ok(PSR_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetPifResponse { pif } => pif
                .to_bytes(buffer)
                .map(|_| 36)
                .map_err(|_| mctp_rs::MctpPacketError::HeaderParseError("Pif parse failed")),
            Self::BatteryGetBpsResponse { bps } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(bps.revision));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(bps.instantaneous_peak_power_level));
                buffer[8..12].copy_from_slice(&u32::to_le_bytes(bps.instantaneous_peak_power_period));
                buffer[12..16].copy_from_slice(&u32::to_le_bytes(bps.sustainable_peak_power_level));
                buffer[16..20].copy_from_slice(&u32::to_le_bytes(bps.sustainable_peak_power_period));

                Ok(BPS_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBtpResponse {} => Ok(0),
            Self::BatterySetBptResponse {} => Ok(0),
            Self::BatteryGetBpcResponse { bpc } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(bpc.revision));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(bpc.power_threshold_support.bits()));
                buffer[8..12].copy_from_slice(&u32::to_le_bytes(bpc.max_instantaneous_peak_power_threshold));
                buffer[12..16].copy_from_slice(&u32::to_le_bytes(bpc.max_sustainable_peak_power_threshold));

                Ok(BPC_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBmcResponse {} => Ok(0),
            Self::BatteryGetBmdResponse { bmd } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(bmd.status_flags.bits()));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(bmd.capability_flags.bits()));
                buffer[8..12].copy_from_slice(&u32::to_le_bytes(bmd.recalibrate_count));
                buffer[12..16].copy_from_slice(&u32::to_le_bytes(bmd.quick_recalibrate_time));
                buffer[16..20].copy_from_slice(&u32::to_le_bytes(bmd.slow_recalibrate_time));

                Ok(BMD_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetBctResponse { bct_response } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(bct_response.into()));

                Ok(BCT_RETURN_SIZE_BYTES)
            }
            Self::BatteryGetBtmResponse { btm_response } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(btm_response.into()));

                Ok(BTM_RETURN_SIZE_BYTES)
            }
            Self::BatterySetBmsResponse { status } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::BatterySetBmaResponse { status } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::BatteryGetStaResponse { sta } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(sta.bits()));

                Ok(STA_RETURN_SIZE_BYTES)
            }
            Self::ThermalGetTmpResponse { temperature } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(temperature));

                Ok(4)
            }
            Self::ThermalSetThrsResponse { status } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::ThermalGetThrsResponse {
                status,
                timeout,
                low,
                high,
            } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(timeout));
                buffer[8..12].copy_from_slice(&u32::to_le_bytes(low));
                buffer[12..16].copy_from_slice(&u32::to_le_bytes(high));

                Ok(16)
            }
            Self::ThermalSetScpResponse { status } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::ThermalGetVarResponse { status, val } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));
                buffer[4..8].copy_from_slice(&u32::to_le_bytes(val));

                Ok(8)
            }
            Self::ThermalSetVarResponse { status } => {
                buffer[..4].copy_from_slice(&u32::to_le_bytes(status));

                Ok(4)
            }
            Self::DebugGetMsgsResponse { debug_buf } => {
                buffer[..debug_buf.len()].copy_from_slice(&debug_buf);
                Ok(debug_buf.len())
            }
            Self::ErrorResponse {} => Ok(0),
        }
    }

    fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
        Ok(match header.command_code {
            OdpCommandCode::BatteryGetBixRequest => Self::BatteryGetBixRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetBstRequest => Self::BatteryGetBstRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetPsrRequest => Self::BatteryGetPsrRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetPifRequest => Self::BatteryGetPifRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetBpsRequest => Self::BatteryGetBpsRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatterySetBtpRequest => Self::BatterySetBtpRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                btp: embedded_batteries_async::acpi::Btp {
                    trip_point: safe_get_dword(buffer, 1)?,
                },
            },
            OdpCommandCode::BatterySetBptRequest => Self::BatterySetBptRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                bpt: embedded_batteries_async::acpi::Bpt {
                    revision: safe_get_dword(buffer, 1)?,
                    threshold_id: match safe_get_dword(buffer, 5)? {
                        0 => embedded_batteries_async::acpi::ThresholdId::ClearAll,
                        1 => embedded_batteries_async::acpi::ThresholdId::InstantaneousPeakPower,
                        2 => embedded_batteries_async::acpi::ThresholdId::SustainablePeakPower,
                        _ => {
                            return Err(MctpPacketError::HeaderParseError("Unsupported threshold id"));
                        }
                    },
                    threshold_value: safe_get_dword(buffer, 9)?,
                },
            },
            OdpCommandCode::BatteryGetBpcRequest => Self::BatteryGetBpcRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatterySetBmcRequest => Self::BatterySetBmcRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                bmc: embedded_batteries_async::acpi::Bmc {
                    maintenance_control_flags: embedded_batteries_async::acpi::BmcControlFlags::from_bits_retain(
                        safe_get_dword(buffer, 1)?,
                    ),
                },
            },
            OdpCommandCode::BatteryGetBmdRequest => Self::BatteryGetBmdRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetBctRequest => Self::BatteryGetBctRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                bct: embedded_batteries_async::acpi::Bct {
                    charge_level_percent: safe_get_dword(buffer, 1)?,
                },
            },
            OdpCommandCode::BatteryGetBtmRequest => Self::BatteryGetBtmRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                btm: embedded_batteries_async::acpi::Btm {
                    discharge_rate: safe_get_dword(buffer, 1)?,
                },
            },
            OdpCommandCode::BatterySetBmsRequest => Self::BatterySetBmsRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                bms: embedded_batteries_async::acpi::Bms {
                    sampling_time_ms: safe_get_dword(buffer, 1)?,
                },
            },
            OdpCommandCode::BatterySetBmaRequest => Self::BatterySetBmaRequest {
                battery_id: safe_get_u8(buffer, 0)?,
                bma: embedded_batteries_async::acpi::Bma {
                    averaging_interval_ms: safe_get_dword(buffer, 1)?,
                },
            },
            OdpCommandCode::BatteryGetStaRequest => Self::BatteryGetStaRequest {
                battery_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::ThermalGetTmpRequest => Self::ThermalGetTmpRequest {
                instance_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::ThermalSetThrsRequest => Self::ThermalSetThrsRequest {
                instance_id: safe_get_u8(buffer, 0)?,
                timeout: safe_get_dword(buffer, 1)?,
                low: safe_get_dword(buffer, 5)?,
                high: safe_get_dword(buffer, 9)?,
            },
            OdpCommandCode::ThermalGetThrsRequest => Self::ThermalGetThrsRequest {
                instance_id: safe_get_u8(buffer, 0)?,
            },
            OdpCommandCode::ThermalSetScpRequest => Self::ThermalSetScpRequest {
                instance_id: safe_get_u8(buffer, 0)?,
                policy_id: safe_get_dword(buffer, 1)?,
                acoustic_lim: safe_get_dword(buffer, 5)?,
                power_lim: safe_get_dword(buffer, 9)?,
            },
            OdpCommandCode::ThermalGetVarRequest => Self::ThermalGetVarRequest {
                instance_id: safe_get_u8(buffer, 0)?,
                len: safe_get_u16(buffer, 1)?,
                var_uuid: safe_get_uuid(buffer, 3)?,
            },
            OdpCommandCode::ThermalSetVarRequest => Self::ThermalSetVarRequest {
                instance_id: safe_get_u8(buffer, 0)?,
                len: safe_get_u16(buffer, 1)?,
                var_uuid: safe_get_uuid(buffer, 3)?,
                set_var: safe_get_dword(buffer, 19)?,
            },
            OdpCommandCode::DebugGetMsgsRequest => Self::DebugGetMsgsRequest,
            OdpCommandCode::BatteryGetBixResponse => Self::BatteryGetBixResponse {
                bix: BixFixedStrings {
                    revision: safe_get_dword(buffer, 0)?,
                    power_unit: match safe_get_dword(buffer, 4)? {
                        0 => embedded_batteries_async::acpi::PowerUnit::MilliWatts,
                        1 => embedded_batteries_async::acpi::PowerUnit::MilliAmps,
                        _ => {
                            return Err(MctpPacketError::HeaderParseError("BIX deserialize failed"));
                        }
                    },
                    design_capacity: safe_get_dword(buffer, 8)?,
                    last_full_charge_capacity: safe_get_dword(buffer, 12)?,
                    battery_technology: match safe_get_dword(buffer, 16)? {
                        0 => embedded_batteries_async::acpi::BatteryTechnology::Primary,
                        1 => embedded_batteries_async::acpi::BatteryTechnology::Secondary,
                        _ => {
                            return Err(MctpPacketError::HeaderParseError("BIX deserialize failed"));
                        }
                    },
                    design_voltage: safe_get_dword(buffer, 20)?,
                    design_cap_of_warning: safe_get_dword(buffer, 24)?,
                    design_cap_of_low: safe_get_dword(buffer, 28)?,
                    cycle_count: safe_get_dword(buffer, 32)?,
                    measurement_accuracy: safe_get_dword(buffer, 36)?,
                    max_sampling_time: safe_get_dword(buffer, 40)?,
                    min_sampling_time: safe_get_dword(buffer, 44)?,
                    max_averaging_interval: safe_get_dword(buffer, 48)?,
                    min_averaging_interval: safe_get_dword(buffer, 52)?,
                    battery_capacity_granularity_1: safe_get_dword(buffer, 56)?,
                    battery_capacity_granularity_2: safe_get_dword(buffer, 60)?,
                    model_number: buffer[64..72]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("BIX deserialize failed"))?,
                    serial_number: buffer[72..80]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("BIX deserialize failed"))?,
                    battery_type: buffer[80..88]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("BIX deserialize failed"))?,
                    oem_info: buffer[88..96]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("BIX deserialize failed"))?,
                    battery_swapping_capability: match safe_get_dword(buffer, 100)? {
                        0 => embedded_batteries_async::acpi::BatterySwapCapability::NonSwappable,
                        1 => embedded_batteries_async::acpi::BatterySwapCapability::ColdSwappable,
                        2 => embedded_batteries_async::acpi::BatterySwapCapability::HotSwappable,
                        _ => {
                            return Err(MctpPacketError::HeaderParseError("BIX deserialize failed"));
                        }
                    },
                },
            },
            OdpCommandCode::BatteryGetBstResponse => Self::BatteryGetBstResponse {
                bst: embedded_batteries_async::acpi::BstReturn {
                    battery_state: BatteryState::from_bits_retain(safe_get_dword(buffer, 0)?),
                    battery_remaining_capacity: safe_get_dword(buffer, 4)?,
                    battery_present_rate: safe_get_dword(buffer, 8)?,
                    battery_present_voltage: safe_get_dword(buffer, 12)?,
                },
            },
            OdpCommandCode::BatteryGetPsrResponse => Self::BatteryGetPsrResponse {
                psr: PsrReturn {
                    power_source: match safe_get_dword(buffer, 0)? {
                        0 => embedded_batteries_async::acpi::PowerSource::Offline,
                        1 => embedded_batteries_async::acpi::PowerSource::Online,
                        _ => {
                            return Err(MctpPacketError::HeaderParseError("PSR deserialize failed"));
                        }
                    },
                },
            },
            OdpCommandCode::BatteryGetPifResponse => Self::BatteryGetPifResponse {
                pif: PifFixedStrings {
                    power_source_state: PowerSourceState::from_bits_retain(safe_get_dword(buffer, 0)?),
                    max_output_power: safe_get_dword(buffer, 4)?,
                    max_input_power: safe_get_dword(buffer, 8)?,
                    model_number: buffer[12..20]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("Pif deserialize failed"))?,
                    serial_number: buffer[20..28]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("Pif deserialize failed"))?,
                    oem_info: buffer[28..36]
                        .try_into()
                        .map_err(|_| MctpPacketError::HeaderParseError("Pif deserialize failed"))?,
                },
            },
            OdpCommandCode::BatteryGetBpsResponse => Self::BatteryGetBpsResponse {
                bps: embedded_batteries_async::acpi::Bps {
                    revision: safe_get_dword(buffer, 0)?,
                    instantaneous_peak_power_level: safe_get_dword(buffer, 4)?,
                    instantaneous_peak_power_period: safe_get_dword(buffer, 8)?,
                    sustainable_peak_power_level: safe_get_dword(buffer, 12)?,
                    sustainable_peak_power_period: safe_get_dword(buffer, 16)?,
                },
            },
            OdpCommandCode::BatterySetBtpResponse => Self::BatterySetBtpResponse {},
            OdpCommandCode::BatterySetBptResponse => Self::BatterySetBptResponse {},
            OdpCommandCode::BatteryGetBpcResponse => Self::BatteryGetBpcResponse {
                bpc: embedded_batteries_async::acpi::Bpc {
                    revision: safe_get_dword(buffer, 0)?,
                    power_threshold_support: PowerThresholdSupport::from_bits_retain(safe_get_dword(buffer, 4)?),
                    max_instantaneous_peak_power_threshold: safe_get_dword(buffer, 8)?,
                    max_sustainable_peak_power_threshold: safe_get_dword(buffer, 12)?,
                },
            },
            OdpCommandCode::BatterySetBmcResponse => Self::BatterySetBmcResponse {},
            OdpCommandCode::BatteryGetBmdResponse => Self::BatteryGetBmdResponse {
                bmd: embedded_batteries_async::acpi::Bmd {
                    status_flags: BmdStatusFlags::from_bits_retain(safe_get_dword(buffer, 0)?),
                    capability_flags: BmdCapabilityFlags::from_bits_retain(safe_get_dword(buffer, 4)?),
                    recalibrate_count: safe_get_dword(buffer, 8)?,
                    quick_recalibrate_time: safe_get_dword(buffer, 12)?,
                    slow_recalibrate_time: safe_get_dword(buffer, 16)?,
                },
            },
            OdpCommandCode::BatteryGetBctResponse => Self::BatteryGetBctResponse {
                bct_response: embedded_batteries_async::acpi::BctReturnResult::from(safe_get_dword(buffer, 0)?),
            },
            OdpCommandCode::BatteryGetBtmResponse => Self::BatteryGetBtmResponse {
                btm_response: embedded_batteries_async::acpi::BtmReturnResult::from(safe_get_dword(buffer, 0)?),
            },
            OdpCommandCode::BatterySetBmsResponse => Self::BatterySetBmsResponse {
                status: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::BatterySetBmaResponse => Self::BatterySetBmaResponse {
                status: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::BatteryGetStaResponse => Self::BatteryGetStaResponse {
                sta: embedded_batteries_async::acpi::StaReturn::from_bits_retain(safe_get_dword(buffer, 0)?),
            },
            OdpCommandCode::ThermalGetTmpResponse => Self::ThermalGetTmpResponse {
                temperature: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::ThermalSetThrsResponse => Self::ThermalSetThrsResponse {
                status: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::ThermalGetThrsResponse => Self::ThermalGetThrsResponse {
                status: safe_get_dword(buffer, 0)?,
                timeout: safe_get_dword(buffer, 4)?,
                low: safe_get_dword(buffer, 8)?,
                high: safe_get_dword(buffer, 12)?,
            },
            OdpCommandCode::ThermalSetScpResponse => Self::ThermalSetScpResponse {
                status: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::ThermalGetVarResponse => Self::ThermalGetVarResponse {
                status: safe_get_dword(buffer, 0)?,
                val: safe_get_dword(buffer, 4)?,
            },
            OdpCommandCode::ThermalSetVarResponse => Self::ThermalSetVarResponse {
                status: safe_get_dword(buffer, 0)?,
            },
            OdpCommandCode::DebugGetMsgsResponse => Self::DebugGetMsgsResponse {
                debug_buf: buffer[..DEBUG_BUF_SIZE]
                    .try_into()
                    .map_err(|_| MctpPacketError::HeaderParseError("MCTP buf not large enough"))?,
            },
        })
    }
}

fn safe_get_u8<M: MctpMedium>(buffer: &[u8], index: usize) -> MctpPacketResult<u8, M> {
    if buffer.len() < index + 1 {
        return Err(MctpPacketError::HeaderParseError("buffer too small for odp message"));
    }
    Ok(buffer[index])
}

fn safe_get_u16<M: MctpMedium>(buffer: &[u8], index: usize) -> MctpPacketResult<u16, M> {
    if buffer.len() < index + 2 {
        return Err(MctpPacketError::HeaderParseError("buffer too small for odp message"));
    }
    // Safe from panics as length is verified above.
    Ok(u16::from_le_bytes(buffer[index..index + 2].try_into().unwrap()))
}

fn safe_get_dword<M: MctpMedium>(buffer: &[u8], index: usize) -> MctpPacketResult<Dword, M> {
    if buffer.len() < index + 4 {
        return Err(MctpPacketError::HeaderParseError("buffer too small for odp message"));
    }
    // Safe from panics as length is verified above.
    Ok(u32::from_le_bytes(buffer[index..index + 4].try_into().unwrap()))
}

fn safe_get_uuid<M: MctpMedium>(buffer: &[u8], index: usize) -> MctpPacketResult<uuid::Bytes, M> {
    if buffer.len() < index + 16 {
        return Err(MctpPacketError::HeaderParseError("buffer too small for odp message"));
    }
    // Safe from panics as length is verified above.
    Ok(buffer[index..index + 16].try_into().unwrap())
}

fn write_to_buffer<M: MctpMedium, const N: usize>(buffer: &mut [u8], data: [u8; N]) -> MctpPacketResult<usize, M> {
    if buffer.len() < N {
        return Err(MctpPacketError::SerializeError("buffer too small for odp message"));
    }
    buffer[..N].copy_from_slice(&data);
    Ok(N)
}

fn check_header_length<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(), M> {
    if buffer.len() < 3 {
        return Err(MctpPacketError::HeaderParseError("buffer too small for odp header"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    mod test_util {
        use mctp_rs::{MctpMedium, MctpMediumFrame, MctpPacketError, error::MctpPacketResult};

        #[derive(Debug, PartialEq, Eq, Copy, Clone)]
        pub struct TestMedium {
            header: &'static [u8],
            trailer: &'static [u8],
            mtu: usize,
        }
        impl TestMedium {
            pub fn new() -> Self {
                Self {
                    header: &[],
                    trailer: &[],
                    mtu: 32,
                }
            }
            pub fn with_headers(mut self, header: &'static [u8], trailer: &'static [u8]) -> Self {
                self.header = header;
                self.trailer = trailer;
                self
            }
            pub fn with_mtu(mut self, mtu: usize) -> Self {
                self.mtu = mtu;
                self
            }
        }

        #[derive(Debug, PartialEq, Eq, Copy, Clone)]
        pub struct TestMediumFrame(usize);

        impl MctpMedium for TestMedium {
            type Frame = TestMediumFrame;
            type Error = &'static str;
            type ReplyContext = ();

            fn deserialize<'buf>(&self, packet: &'buf [u8]) -> MctpPacketResult<(Self::Frame, &'buf [u8]), Self> {
                let packet_len = packet.len();

                // check that header / trailer is present and correct
                if packet.len() < self.header.len() + self.trailer.len() {
                    return Err(MctpPacketError::MediumError("packet too short"));
                }
                if packet[0..self.header.len()] != *self.header {
                    return Err(MctpPacketError::MediumError("header mismatch"));
                }
                if packet[packet_len - self.trailer.len()..packet_len] != *self.trailer {
                    return Err(MctpPacketError::MediumError("trailer mismatch"));
                }

                let packet = &packet[self.header.len()..packet_len - self.trailer.len()];
                Ok((TestMediumFrame(packet_len), packet))
            }
            fn max_message_body_size(&self) -> usize {
                self.mtu
            }
            fn serialize<'buf, F>(
                &self,
                _: Self::ReplyContext,
                buffer: &'buf mut [u8],
                message_writer: F,
            ) -> MctpPacketResult<&'buf [u8], Self>
            where
                F: for<'a> FnOnce(&'a mut [u8]) -> MctpPacketResult<usize, Self>,
            {
                let header_len = self.header.len();
                let trailer_len = self.trailer.len();

                // Ensure buffer can fit at least headers and trailers
                if buffer.len() < header_len + trailer_len {
                    return Err(MctpPacketError::MediumError("Buffer too small for headers"));
                }

                // Calculate available space for message (respecting MTU)
                let max_packet_size = self.mtu.min(buffer.len());
                if max_packet_size < header_len + trailer_len {
                    return Err(MctpPacketError::MediumError("MTU too small for headers"));
                }
                let max_message_size = max_packet_size - header_len - trailer_len;

                buffer[0..header_len].copy_from_slice(self.header);
                let size = message_writer(&mut buffer[header_len..header_len + max_message_size])?;
                let len = header_len + size;
                buffer[len..len + trailer_len].copy_from_slice(self.trailer);
                Ok(&buffer[..len + trailer_len])
            }
        }

        impl MctpMediumFrame<TestMedium> for TestMediumFrame {
            fn packet_size(&self) -> usize {
                self.0
            }
            fn reply_context(&self) -> <TestMedium as MctpMedium>::ReplyContext {}
        }
    }

    use test_util::TestMedium;

    #[rstest::rstest]
    #[case(OdpHeader {
        request_bit: true,
        datagram_bit: false,
        service: OdpService::Battery,
        command_code: OdpCommandCode::BatteryGetBixRequest,
        completion_code: MctpCompletionCode::Success
    })]
    #[case(
        OdpHeader {
        request_bit: false,
        datagram_bit: true,
                service: OdpService::Debug,
        command_code: OdpCommandCode::BatteryGetBixRequest,
        completion_code: MctpCompletionCode::ErrorUnsupportedCmd
    })]
    #[case(
        OdpHeader {
        request_bit: true,
        datagram_bit: true,
        service: OdpService::Battery,
        command_code: OdpCommandCode::BatteryGetBixRequest,
        completion_code: MctpCompletionCode::CommandSpecific(0x80)
    })]
    #[case(
        OdpHeader {
        request_bit: false,
        datagram_bit: false,
        service: OdpService::Debug,
        command_code: OdpCommandCode::BatteryGetBixRequest,
        completion_code: MctpCompletionCode::Success
    })]
    fn odp_header_roundtrip_happy_path(#[case] header: OdpHeader) {
        let mut buf = [0u8; 3];
        let size = header.serialize::<TestMedium>(&mut buf).unwrap();
        assert_eq!(size, 3);

        let (parsed, rest) = OdpHeader::deserialize::<TestMedium>(&buf).unwrap();
        assert_eq!(parsed, header);
        assert_eq!(rest.len(), 0);
    }

    #[test]
    #[allow(clippy::panic)]
    fn odp_header_error_on_short_buffer() {
        let header = OdpHeader {
            request_bit: false,
            datagram_bit: false,
            service: OdpService::Battery,
            command_code: OdpCommandCode::BatteryGetBixRequest,
            completion_code: MctpCompletionCode::Success,
        };

        // Serialize works with correct buffer
        let mut buf_ok = [0u8; 3];
        header.serialize::<TestMedium>(&mut buf_ok).unwrap();

        // Deserialize should fail on too-small buffer
        let err = OdpHeader::deserialize::<TestMedium>(&buf_ok[..2]).unwrap_err();
        match err {
            MctpPacketError::HeaderParseError(msg) => {
                assert_eq!(msg, "buffer too small for odp header")
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
