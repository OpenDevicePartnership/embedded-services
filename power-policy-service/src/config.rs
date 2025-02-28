//! Configuration types for the power policy service

use embedded_services::power::policy::PowerCapability;

#[derive(Clone, Copy)]
pub struct Config {
    /// Above this threshold, the system will be in high power mode
    pub high_power_threshold_mw: u32,
    /// Power capability of every provider in low power mode
    pub provider_low: PowerCapability,
    /// Power capability of every provider in high power mode
    pub provider_high: PowerCapability,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Type-C 5V@3A
            high_power_threshold_mw: 15000,
            // Type-C 5V@3A
            provider_low: PowerCapability {
                voltage_mv: 5000,
                current_ma: 3000,
            },
            // Type-C 5V@1A5
            provider_high: PowerCapability {
                voltage_mv: 5000,
                current_ma: 1500,
            },
        }
    }
}
