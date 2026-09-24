use embedded_usb_pd::{LocalPortId, PdError};

use crate::controller::pd::Pd;

/// Functionality related to setting the maximum sink voltage for a port.
pub trait MaxSinkVoltage: Pd {
    /// Set the maximum sink voltage for the given port
    ///
    /// This may trigger a renegotiation
    fn set_max_sink_voltage(
        &mut self,
        port: LocalPortId,
        voltage_mv: Option<u16>,
    ) -> impl Future<Output = Result<(), PdError>>;
}
