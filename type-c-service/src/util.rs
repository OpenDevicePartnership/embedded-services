//! Type-C service utility functions and constants.
use embedded_usb_pd::{Error as PdBusError, PdError};
use fw_update_interface::basic::Error as BasicFwError;

/// Converts a PD error into a basic FW update error
pub fn basic_fw_update_error_from_pd_error(pd_error: PdError) -> BasicFwError {
    match pd_error {
        PdError::Busy => BasicFwError::Busy,
        _ => BasicFwError::Failed,
    }
}

/// Converts a PD error into a basic FW update error
pub fn basic_fw_update_error_from_pd_bus_error<BE>(pd_error: PdBusError<BE>) -> BasicFwError {
    match pd_error {
        PdBusError::Pd(pd_error) => basic_fw_update_error_from_pd_error(pd_error),
        PdBusError::Bus(_) => BasicFwError::Bus,
    }
}
