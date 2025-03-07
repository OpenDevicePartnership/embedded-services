//! Event definitions
use core::ops::BitOr;

use bitfield::{Bit, BitMut};
use bitflags::bitflags;

/// Port event kind
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortEventKind(pub u32);

bitflags! {
    impl PortEventKind: u32 {
         /// None
        const NONE = 0;
        /// Plug inserted or removed
        const PLUG_INSERTED_OR_REMOVED = (1 << 0);
        /// New contract as consumer
        const NEW_POWER_CONTRACT_AS_CONSUMER = (1 << 3);
    }
}

impl PortEventKind {
    /// Returns true if a plug was inserted or removed
    pub fn plug_inserted_or_removed(self) -> bool {
        self & Self::PLUG_INSERTED_OR_REMOVED != Self::NONE
    }

    /// Returns true if a new power contract was established as consumer
    pub fn new_power_contract_as_consumer(self) -> bool {
        self & Self::NEW_POWER_CONTRACT_AS_CONSUMER != Self::NONE
    }
}

/// Bit vector to store which ports have unhandled events
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(transparent)]
pub struct PortEventFlags(pub u32);

impl PortEventFlags {
    /// Returns the number of bits in the event
    pub const fn len(&self) -> usize {
        size_of::<Self>() * 8
    }
}

impl BitMut for PortEventFlags {
    fn set_bit(&mut self, bit: usize, value: bool) {
        self.0.set_bit(bit, value);
    }
}

impl Bit for PortEventFlags {
    fn bit(&self, bit: usize) -> bool {
        self.0.bit(bit)
    }
}

impl BitOr for PortEventFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        PortEventFlags(self.0 | rhs.0)
    }
}
