use embedded_usb_pd::ucsi;

/// Type-c service configuration
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    /// UCSI capabilities
    pub ucsi_capabilities: ucsi::ppm::get_capability::ResponseData,
    /// Optional override for UCSI port capabilities
    pub ucsi_port_capabilities: Option<ucsi::lpm::get_connector_capability::ResponseData>,
}
