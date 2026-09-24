use embedded_usb_pd::PdError;

use crate::port::pd::Pd;

/// Functionality related to setting the maximum sink voltage for a port.
pub trait MaxSinkVoltage: Pd {
    /// Set the maximum sink voltage for this port
    ///
    /// This may trigger a renegotiation
    fn set_max_sink_voltage(&mut self, voltage_mv: Option<u16>) -> impl Future<Output = Result<(), PdError>>;
}
