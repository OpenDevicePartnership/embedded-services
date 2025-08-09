use embedded_usb_pd::ucsi;

/// Type-c service configuration
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    /// UCSI capabilities
    pub ucsi_capabilities: ucsi::ppm::get_capability::ResponseData,
}
