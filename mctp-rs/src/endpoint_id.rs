use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EndpointId {
    /// 0x00
    #[default]
    Null,
    /// 0xFF
    Broadcast,
    /// 0x08 - 0x7F
    Id(u8),
}

impl TryFrom<u8> for EndpointId {
    type Error = &'static str;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_from_bits(value as u32)
    }
}

impl TryFromBits<u32> for EndpointId {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        match bits {
            0x00 => Ok(EndpointId::Null),
            0xFF => Ok(EndpointId::Broadcast),
            0x08..=0xFE => Ok(EndpointId::Id(bits as u8)),
            _ => Err("Invalid endpoint ID"),
        }
    }
}

impl TryIntoBits<u32> for EndpointId {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        match self {
            EndpointId::Null => Ok(0x00),
            EndpointId::Broadcast => Ok(0xFF),
            EndpointId::Id(id) => Ok(id as u32),
        }
    }
}

impl NumBytes for EndpointId {
    const NUM_BYTES: usize = 1;
}
