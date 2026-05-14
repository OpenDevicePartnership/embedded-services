//! Module for PD controller related code

use embedded_services::named::Named;
use embedded_usb_pd::PdError;

pub mod electrical_disconnect;
pub mod max_sink_voltage;
pub mod pd;
pub mod power;
pub mod retimer;
pub mod type_c;

/// Controller ID
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerId(pub u8);

/// PD controller trait
pub trait Controller: Named {
    /// Reset the controller
    fn reset_controller(&mut self) -> impl Future<Output = Result<(), PdError>>;
}
