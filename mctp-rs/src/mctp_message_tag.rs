use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpMessageTag(u8);

impl TryFrom<u8> for MctpMessageTag {
    type Error = &'static str;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > 0b111 {
            return Err("Invalid message tag");
        }
        Ok(Self(value))
    }
}

impl NumBytes for MctpMessageTag {
    const NUM_BYTES: usize = 1;
}

impl TryFromBits<u32> for MctpMessageTag {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        Self::try_from(bits as u8)
    }
}

impl TryIntoBits<u32> for MctpMessageTag {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(self.0 as u32)
    }
}
