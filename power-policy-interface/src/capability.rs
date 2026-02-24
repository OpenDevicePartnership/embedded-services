//! Power capability definitions and related flags
use bitfield::bitfield;
use num_enum::{IntoPrimitive, TryFromPrimitive};

/// Amount of power that a device can provider or consume
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PowerCapability {
    /// Available voltage in mV
    pub voltage_mv: u16,
    /// Max available current in mA
    pub current_ma: u16,
}

impl PowerCapability {
    /// Calculate maximum power
    pub fn max_power_mw(&self) -> u32 {
        self.voltage_mv as u32 * self.current_ma as u32 / 1000
    }
}

impl PartialOrd for PowerCapability {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PowerCapability {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.max_power_mw().cmp(&other.max_power_mw())
    }
}

/// Power capability with consumer flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConsumerPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Consumer flags
    pub flags: ConsumerFlags,
}

impl From<PowerCapability> for ConsumerPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: ConsumerFlags::none(),
        }
    }
}

/// Power capability with provider flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProviderPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Provider flags
    pub flags: ProviderFlags,
}

impl From<PowerCapability> for ProviderPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: ProviderFlags::none(),
        }
    }
}

/// Combined power capability with flags enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerCapabilityFlags {
    /// Consumer flags
    Consumer(ConsumerPowerCapability),
    /// Provider flags
    Provider(ProviderPowerCapability),
}

/// PSU type
#[derive(Copy, Clone, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[num_enum(error_type(name = InvalidPsuType, constructor = InvalidPsuType))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
#[non_exhaustive]
pub enum PsuType {
    /// Unknown/Unspecified
    Unknown,
    /// Type-C port
    TypeC,
    /// DC barrel jack
    DcJack,

    /// Application defined type
    Custom0 = 12,
    /// Application defined type
    Custom1 = 13,
    /// Application defined type
    Custom2 = 14,
    /// Application defined type
    Custom3 = 15,
    // End to fit into 4 bits
}

/// Conversion error for [`PsuType`]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InvalidPsuType(pub u8);

bitfield! {
    /// Raw consumer flags bit field
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct ConsumerFlagsRaw(u32);
    impl Debug;
    /// Unconstrained power, indicates that we are drawing power from something like an outlet and not a limited source like a battery
    pub bool, unconstrained_power, set_unconstrained_power: 0;
    /// PSU type
    pub u8, psu_type, set_psu_type: 11, 8;
}

/// Type safe wrapper for consumer flags
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConsumerFlags(ConsumerFlagsRaw);

impl ConsumerFlags {
    /// Create a new consumer with no flags set
    pub const fn none() -> Self {
        Self(ConsumerFlagsRaw(0))
    }

    /// Builder method to set the unconstrained power flag
    pub fn with_unconstrained_power(mut self) -> Self {
        self.0.set_unconstrained_power(true);
        self
    }

    /// Check if the unconstrained power flag is set
    pub fn unconstrained_power(&self) -> bool {
        self.0.unconstrained_power()
    }

    /// Set the unconstrained power flag
    pub fn set_unconstrained_power(&mut self, value: bool) {
        self.0.set_unconstrained_power(value);
    }

    /// Builder method to set the PSU type
    pub fn with_psu_type(mut self, value: PsuType) -> Self {
        self.set_psu_type(value);
        self
    }

    /// Return PSU type
    pub fn psu_type(&self) -> PsuType {
        PsuType::try_from(self.0.psu_type()).unwrap_or(PsuType::Unknown)
    }

    /// Set PSU type
    pub fn set_psu_type(&mut self, value: PsuType) {
        self.0.set_psu_type(value as u8);
    }
}

bitfield! {
    /// Raw provider flags bit field
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct ProviderRaw(u32);
    impl Debug;
    /// PSU type
    pub u8, psu_type, set_psu_type: 11, 8;
}

/// Type safe wrapper for provider flags
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProviderFlags(ProviderRaw);

impl ProviderFlags {
    /// Create a new provider with no flags set
    pub const fn none() -> Self {
        Self(ProviderRaw(0))
    }

    /// Builder method to set the PSU type
    pub fn with_psu_type(mut self, value: PsuType) -> Self {
        self.set_psu_type(value);
        self
    }

    /// Return PSU type
    pub fn psu_type(&self) -> PsuType {
        PsuType::try_from(self.0.psu_type()).unwrap_or(PsuType::Unknown)
    }

    /// Set PSU type
    pub fn set_psu_type(&mut self, value: PsuType) {
        self.0.set_psu_type(value as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_psu_type_conversion() {
        // Test valid conversions
        assert_eq!(PsuType::try_from(u8::from(PsuType::TypeC)), Ok(PsuType::TypeC));
        assert_eq!(PsuType::try_from(u8::from(PsuType::DcJack)), Ok(PsuType::DcJack));
        assert_eq!(PsuType::try_from(u8::from(PsuType::Custom0)), Ok(PsuType::Custom0));
        assert_eq!(PsuType::try_from(u8::from(PsuType::Custom1)), Ok(PsuType::Custom1));
        assert_eq!(PsuType::try_from(u8::from(PsuType::Custom2)), Ok(PsuType::Custom2));
        assert_eq!(PsuType::try_from(u8::from(PsuType::Custom3)), Ok(PsuType::Custom3));
        assert_eq!(PsuType::try_from(u8::from(PsuType::Unknown)), Ok(PsuType::Unknown));

        assert_eq!(PsuType::try_from(3), Err(InvalidPsuType(3)));
        assert_eq!(PsuType::try_from(4), Err(InvalidPsuType(4)));
        assert_eq!(PsuType::try_from(5), Err(InvalidPsuType(5)));
        assert_eq!(PsuType::try_from(6), Err(InvalidPsuType(6)));
        assert_eq!(PsuType::try_from(7), Err(InvalidPsuType(7)));
        assert_eq!(PsuType::try_from(8), Err(InvalidPsuType(8)));
        assert_eq!(PsuType::try_from(9), Err(InvalidPsuType(9)));
        assert_eq!(PsuType::try_from(10), Err(InvalidPsuType(10)));
        assert_eq!(PsuType::try_from(11), Err(InvalidPsuType(11)));

        for i in 16..=255 {
            assert_eq!(PsuType::try_from(i), Err(InvalidPsuType(i)));
        }
    }

    #[test]
    fn test_consumer_flags_unconstrained() {
        let mut consumer = ConsumerFlags::none().with_unconstrained_power();
        assert_eq!(consumer.0.0, 0x1);
        consumer.set_unconstrained_power(false);
        assert_eq!(consumer.0.0, 0x0);
    }

    #[test]
    fn test_consumer_flags_psu_type() {
        let mut consumer = ConsumerFlags::none().with_psu_type(PsuType::TypeC);
        assert_eq!(consumer.0.0, 0x100);
        consumer.set_psu_type(PsuType::Unknown);
        assert_eq!(consumer.0.0, 0x0);
    }

    #[test]
    fn test_provider_flags_psu_type() {
        let mut provider = ProviderFlags::none().with_psu_type(PsuType::TypeC);
        assert_eq!(provider.0.0, 0x100);
        provider.set_psu_type(PsuType::Unknown);
        assert_eq!(provider.0.0, 0x0);
    }
}
