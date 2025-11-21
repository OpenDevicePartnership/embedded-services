//! Configuration types for the power policy service

use embedded_services::power::policy::PowerCapability;

#[derive(Clone, Copy)]
pub struct Config {
    /// Above this threshold, the system is in limited power mode
    pub limited_power_threshold_mw: u32,
    /// Power capability of every provider in normal power mode
    pub provider_unlimited: PowerCapability,
    /// Power capability of every provider in limited power mode
    pub provider_limited: PowerCapability,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Type-C 5V@3A
            limited_power_threshold_mw: 15000,
            // Type-C 5V@3A
            provider_unlimited: PowerCapability {
                voltage_mv: 5000,
                current_ma: 3000,
            },
            // Type-C 5V@1A5
            provider_limited: PowerCapability {
                voltage_mv: 5000,
                current_ma: 1500,
            },
        }
    }
}
