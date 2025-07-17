//! Consumer and provider flags, these are used to signal additional information about a consumer/provider request

use bitfield::bitfield;

bitfield! {
    /// Raw consumer flags bit field
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct ConsumerRaw(u32);
    impl Debug;
    /// Unconstrained power, indicates that we are drawing power from something like an outlet and not a limited source like a battery
    pub u8, unconstrained_power, set_unconstrained_power: 0, 0;
}

/// Type safe wrapper for consumer flags
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Consumer(ConsumerRaw);

impl Consumer {
    /// Create a new consumer with no flags set
    pub const fn none() -> Self {
        Self(ConsumerRaw(0))
    }

    /// Builder method to set the unconstrained power flag
    pub fn with_unconstrained_power(mut self) -> Self {
        self.0.set_unconstrained_power(1);
        self
    }

    /// Check if the unconstrained power flag is set
    pub fn unconstrained_power(&self) -> bool {
        self.0.unconstrained_power() != 0
    }

    /// Set the unconstrained power flag
    pub fn set_unconstrained_power(&mut self, value: bool) {
        self.0.set_unconstrained_power(value as u8);
    }
}

/// Type safe wrapper for provider flags
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Provider(());

impl Provider {
    /// Create a new provider with no flags set
    pub const fn none() -> Self {
        Self(())
    }
}
