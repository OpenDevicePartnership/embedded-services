use embedded_usb_pd::ucsi::{self, lpm::get_connector_status::BatteryChargingCapabilityStatus};

/// UCSI battery charging capability status configuration.
///
/// This struct holds the power thresholds for determining the battery charging capability status as reported by the
/// UCSI `GET_CONNECTOR_STATUS` command.
///
/// See [`try_new`][`Self::try_new`] for details on creating a valid configuration, and [`status_of`][`Self::status_of`]
/// for determining the status based on a power level.
///
/// The [`Default`][`Self::default`] implementation creates a configuration where the status of all power levels is
/// considered [`Nominal`][BatteryChargingCapabilityStatus::Nominal].
#[derive(Debug, Clone, Copy, Default)]
pub struct UcsiBatteryChargingThresholdConfig {
    /// Power threshold (in milliwatts) to be considered not charging.
    ///
    /// Below this level, `GET_CONNECTOR_STATUS` will report [`NotCharging`][BatteryChargingCapabilityStatus::NotCharging].
    not_charging_mw: Option<u32>,

    /// Power threshold (in milliwatts) to be considered very slow charging.
    ///
    /// Below this level, `GET_CONNECTOR_STATUS` will report [`VerySlow`][BatteryChargingCapabilityStatus::VerySlow].
    very_slow_mw: Option<u32>,

    /// Power threshold (in milliwatts) to be considered slow charging.
    ///
    /// Below this level, `GET_CONNECTOR_STATUS` will report [`Slow`][BatteryChargingCapabilityStatus::Slow].
    slow_mw: Option<u32>,
}

impl UcsiBatteryChargingThresholdConfig {
    /// Create a new [`UcsiBatteryChargingThresholdConfig`], ensuring the exclusive thresholds are in the correct order.
    ///
    /// The thresholds must satisfy:
    ///
    /// ```text
    /// not_charging_mw < very_slow_charging_mw < slow_charging_mw
    /// ```
    ///
    /// Any of the thresholds can be [`None`], which ignores that threshold in the ordering checks and subsequently when
    /// determining the status in [`status_of`][`Self::status_of`].
    ///
    /// Returns [`None`] if the thresholds are misordered.
    pub const fn try_new(
        not_charging_mw: Option<u32>,
        very_slow_charging_mw: Option<u32>,
        slow_charging_mw: Option<u32>,
    ) -> Option<Self> {
        if let (Some(not), Some(very_slow)) = (not_charging_mw, very_slow_charging_mw)
            && not >= very_slow
        {
            return None;
        };

        if let (Some(very_slow), Some(slow)) = (very_slow_charging_mw, slow_charging_mw)
            && very_slow >= slow
        {
            return None;
        };

        if let (Some(not), Some(slow)) = (not_charging_mw, slow_charging_mw)
            && not >= slow
        {
            return None;
        };

        Some(Self {
            not_charging_mw,
            very_slow_mw: very_slow_charging_mw,
            slow_mw: slow_charging_mw,
        })
    }

    /// Compare a power level (in milliwatts) against the exclusive thresholds and return the corresponding status.
    ///
    /// If below a threshold, that status is returned. If a threshold is [`None`], it is ignored and its status won't be
    /// returned. The order of checks is from lowest to highest threshold:
    /// 1. `not_charging_mw` -> [`NotCharging`][BatteryChargingCapabilityStatus::NotCharging]
    /// 1. `very_slow_charging_mw` -> [`VerySlow`][BatteryChargingCapabilityStatus::VerySlow]
    /// 1. `slow_charging_mw` -> [`Slow`][BatteryChargingCapabilityStatus::Slow]
    /// 1. Above all thresholds -> [`Nominal`][BatteryChargingCapabilityStatus::Nominal]
    pub const fn status_of(&self, power_mw: u32) -> BatteryChargingCapabilityStatus {
        if let Some(threshold) = self.not_charging_mw
            && power_mw < threshold
        {
            BatteryChargingCapabilityStatus::NotCharging
        } else if let Some(threshold) = self.very_slow_mw
            && power_mw < threshold
        {
            BatteryChargingCapabilityStatus::VerySlow
        } else if let Some(threshold) = self.slow_mw
            && power_mw < threshold
        {
            BatteryChargingCapabilityStatus::Slow
        } else {
            BatteryChargingCapabilityStatus::Nominal
        }
    }
}

/// Type-c service configuration
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    /// UCSI capabilities
    pub ucsi_capabilities: ucsi::ppm::get_capability::ResponseData,
    /// Optional override for UCSI port capabilities
    pub ucsi_port_capabilities: Option<ucsi::lpm::get_connector_capability::ResponseData>,
    /// UCSI battery charging configuration
    pub ucsi_battery_charging_config: UcsiBatteryChargingThresholdConfig,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    mod ucsi_battery_charging_threshold_config {
        //! Tests for [`UcsiBatteryChargingThresholdConfig`]

        use super::*;

        mod try_new {
            //! Tests for [`UcsiBatteryChargingThresholdConfig::try_new`]

            use super::*;

            #[test]
            fn valid() {
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(3000)).is_some());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), Some(3000)).is_some());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, Some(3000)).is_some());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), None).is_some());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(None, None, None).is_some());
            }

            #[test]
            fn invalid() {
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(3000), Some(2000), Some(1000)).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(2000), Some(2000), Some(3000)).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(1000), Some(3000)).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(2000)).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(3000), None, Some(1000)).is_none());
            }

            #[test]
            fn equal_is_invalid() {
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(1000), None).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), Some(2000)).is_none());
                assert!(UcsiBatteryChargingThresholdConfig::try_new(Some(3000), None, Some(3000)).is_none());
            }
        }

        /// Test that the default config permits any power level to be nominal.
        #[test]
        fn default() {
            let config = UcsiBatteryChargingThresholdConfig::default();
            assert_eq!(config.status_of(0), BatteryChargingCapabilityStatus::Nominal);
        }

        mod status_of {
            //! Tests for [`UcsiBatteryChargingThresholdConfig::status_of`]

            use super::*;

            #[test]
            fn not_charging() {
                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(999), BatteryChargingCapabilityStatus::NotCharging);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, None).unwrap();
                assert_eq!(config.status_of(999), BatteryChargingCapabilityStatus::NotCharging);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), None).unwrap();
                assert_eq!(config.status_of(999), BatteryChargingCapabilityStatus::NotCharging);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, Some(3000)).unwrap();
                assert_eq!(config.status_of(999), BatteryChargingCapabilityStatus::NotCharging);
            }

            #[test]
            fn very_slow() {
                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(1999), BatteryChargingCapabilityStatus::VerySlow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), None).unwrap();
                assert_eq!(config.status_of(1999), BatteryChargingCapabilityStatus::VerySlow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), None).unwrap();
                assert_eq!(config.status_of(1999), BatteryChargingCapabilityStatus::VerySlow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(1999), BatteryChargingCapabilityStatus::VerySlow);
            }

            #[test]
            fn slow() {
                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(2999), BatteryChargingCapabilityStatus::Slow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, None, Some(3000)).unwrap();
                assert_eq!(config.status_of(2999), BatteryChargingCapabilityStatus::Slow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(2999), BatteryChargingCapabilityStatus::Slow);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, Some(3000)).unwrap();
                assert_eq!(config.status_of(2999), BatteryChargingCapabilityStatus::Slow);
            }

            #[test]
            fn nominal() {
                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(3001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, None, None).unwrap();
                assert_eq!(config.status_of(0), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, None).unwrap();
                assert_eq!(config.status_of(1001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), None).unwrap();
                assert_eq!(config.status_of(2001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, None, Some(3000)).unwrap();
                assert_eq!(config.status_of(3001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), Some(2000), None).unwrap();
                assert_eq!(config.status_of(2001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(None, Some(2000), Some(3000)).unwrap();
                assert_eq!(config.status_of(3001), BatteryChargingCapabilityStatus::Nominal);

                let config = UcsiBatteryChargingThresholdConfig::try_new(Some(1000), None, Some(3000)).unwrap();
                assert_eq!(config.status_of(3001), BatteryChargingCapabilityStatus::Nominal);
            }
        }
    }
}
