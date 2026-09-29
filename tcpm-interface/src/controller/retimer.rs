use embedded_services::named::Named;
use embedded_usb_pd::{LocalPortId, PdError};

use crate::control::retimer::RetimerFwUpdateState;

/// Retimer-related functionality
pub trait Retimer: Named {
    /// Returns the retimer fw update state
    fn get_rt_fw_update_status(
        &mut self,
        port: LocalPortId,
    ) -> impl Future<Output = Result<RetimerFwUpdateState, PdError>>;
    /// Set retimer fw update state
    fn set_rt_fw_update_state(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), PdError>>;
    /// Clear retimer fw update state
    fn clear_rt_fw_update_state(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), PdError>>;
    /// Set retimer compliance
    fn set_rt_compliance(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), PdError>>;
    /// Reconfigure the retimer for the given port.
    fn reconfigure_retimer(&mut self, port: LocalPortId) -> impl Future<Output = Result<(), PdError>>;
}
