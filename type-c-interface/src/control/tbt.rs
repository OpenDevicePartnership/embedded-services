//! Thunderbolt-related control types

/// Thunderbolt control configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct TbtConfig {
    /// Enable Thunderbolt
    pub tbt_enabled: bool,
}
