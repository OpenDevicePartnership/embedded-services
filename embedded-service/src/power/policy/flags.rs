//! Consumer and provider flags, these are used to signal additional information about a consumer/provider request

use bitfield::bitfield;

/// PSU type
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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

impl TryFrom<u8> for PsuType {
    type Error = InvalidPsuType;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::TypeC),
            2 => Ok(Self::DcJack),
            12 => Ok(Self::Custom0),
            13 => Ok(Self::Custom1),
            14 => Ok(Self::Custom2),
            15 => Ok(Self::Custom3),
            value => Err(InvalidPsuType(value)),
        }
    }
}

impl From<PsuType> for u8 {
    fn from(value: PsuType) -> Self {
        value as u8
    }
}

bitfield! {
    /// Raw consumer flags bit field
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct ConsumerRaw(u32);
    impl Debug;
    /// Unconstrained power, indicates that we are drawing power from something like an outlet and not a limited source like a battery
    pub bool, unconstrained_power, set_unconstrained_power: 0;
    /// PSU type
    pub u8, psu_type, set_psu_type: 11, 8;
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
pub struct Provider(ProviderRaw);

impl Provider {
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
    fn test_consumer_unconstrained() {
        let mut consumer = Consumer::none().with_unconstrained_power();
        assert_eq!(consumer.0.0, 0x1);
        consumer.set_unconstrained_power(false);
        assert_eq!(consumer.0.0, 0x0);
    }

    #[test]
    fn test_consumer_psu_type() {
        let mut consumer = Consumer::none().with_psu_type(PsuType::TypeC);
        assert_eq!(consumer.0.0, 0x100);
        consumer.set_psu_type(PsuType::Unknown);
        assert_eq!(consumer.0.0, 0x0);
    }

    #[test]
    fn test_provider_psu_type() {
        let mut provider = Provider::none().with_psu_type(PsuType::TypeC);
        assert_eq!(provider.0.0, 0x100);
        provider.set_psu_type(PsuType::Unknown);
        assert_eq!(provider.0.0, 0x0);
    }
}
