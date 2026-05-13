//! USB related control types

/// USB control configuration
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbControlConfig {
    /// Enable USB2 data path
    pub usb2_enabled: bool,
    /// Enable USB3 data path  
    pub usb3_enabled: bool,
    /// Enable USB4 data path
    pub usb4_enabled: bool,
}

impl Default for UsbControlConfig {
    fn default() -> Self {
        Self {
            usb2_enabled: true,
            usb3_enabled: true,
            usb4_enabled: true,
        }
    }
}
