use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MctpSequenceNumber(u8);

impl MctpSequenceNumber {
    const MAX: u8 = 4;

    pub fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn inc(&mut self) -> Self {
        *self = self.next();
        *self
    }

    pub fn next(&self) -> Self {
        Self((self.0 + 1) % Self::MAX)
    }
}

impl NumBytes for MctpSequenceNumber {
    const NUM_BYTES: usize = 1;
}

impl TryIntoBits<u32> for MctpSequenceNumber {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(self.0 as u32)
    }
}

impl TryFromBits<u32> for MctpSequenceNumber {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits >= Self::MAX as u32 {
            Err("sequence number out of range")
        } else {
            Ok(Self(bits as u8))
        }
    }
}
