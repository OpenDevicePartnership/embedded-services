use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Zero;

impl TryFromBits<u32> for Zero {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits != 0 { Err("Bits must be 0") } else { Ok(Zero) }
    }
}
impl TryIntoBits<u32> for Zero {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(0)
    }
}
impl NumBytes for Zero {
    const NUM_BYTES: usize = 4;
}

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct One;

impl TryFromBits<u32> for One {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits != 1 { Err("Value must be 1") } else { Ok(One) }
    }
}
impl TryIntoBits<u32> for One {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(1)
    }
}
impl NumBytes for One {
    const NUM_BYTES: usize = 4;
}
