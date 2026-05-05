use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[repr(u8)]
#[derive(
    Copy, Clone, PartialEq, Eq, Debug, Default, num_enum::IntoPrimitive, num_enum::TryFromPrimitive,
)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpControlCommandCode {
    #[default]
    Reserved = 0x00,
    SetEndpointId = 0x01,
    GetEndpointId = 0x02,
    GetEndpointUuid = 0x03,
    GetMctpVersionSupport = 0x04,
    GetMessageTypeSupport = 0x05,
    GetVendorDefinedMessageSupport = 0x06,
    ResolveEndpointId = 0x07,
    AllocateEndpointIds = 0x08,
    RoutingInformationUpdate = 0x09,
    GetRoutingTableEntries = 0x0A,
    PrepareForEndpointDiscovery = 0x0B,
    EndpointDiscovery = 0x0C,
    DiscoveryNotify = 0x0D,
    GetNetworkId = 0x0E,
    QueryHop = 0x0F,
    ResolveUuid = 0x10,
    QueryRateLimit = 0x11,
    RequestTxRateLimit = 0x12,
    UpdateRateLimit = 0x13,
    QuerySupportedInterfaces = 0x14,
    // 0x15-0xEF are reserved for future use
    // 0xF0-0xFF are transport specific commands
}

impl TryFromBits<u32> for MctpControlCommandCode {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits > 0xFF {
            return Err("Out of range value for MCTP command code");
        }
        (bits as u8)
            .try_into()
            .map_err(|_| "Invalid value for MCTP command code")
    }
}

impl TryIntoBits<u32> for MctpControlCommandCode {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(Into::<u8>::into(self) as u32)
    }
}

impl NumBytes for MctpControlCommandCode {
    const NUM_BYTES: usize = 1;
}
