//! DP-related control types
/// DisplayPort pin configuration
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpPinConfig {
    /// 4L DP connection using USBC-USBC cable (Pin Assignment C)
    pub pin_c: bool,
    /// 2L USB + 2L DP connection using USBC-USBC cable (Pin Assignment D)
    pub pin_d: bool,
    /// 4L DP connection using USBC-DP cable (Pin Assignment E)
    pub pin_e: bool,
}

/// DisplayPort status data
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpStatus {
    /// DP alt-mode entered
    pub alt_mode_entered: bool,
    /// Get DP DFP pin config
    pub dfp_d_pin_cfg: DpPinConfig,
}

/// DisplayPort configuration data
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DpConfig {
    /// DP alt-mode enabled
    pub enable: bool,
    /// Set DP DFP pin config
    pub dfp_d_pin_cfg: DpPinConfig,
}
