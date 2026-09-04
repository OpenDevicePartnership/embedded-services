use embedded_services::named::Named;
use embedded_usb_pd::{PdError, ucsi::v1_2::lpm};

/// UCSI LPM command execution trait
pub trait Lpm: Named {
    /// Execute the given LPM command
    fn execute_lpm_command(
        &mut self,
        command: lpm::LocalCommand,
    ) -> impl Future<Output = Result<Option<lpm::ResponseData>, PdError>>;
}
