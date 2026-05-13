use embedded_usb_pd::PdError;
use embedded_usb_pd::ucsi::lpm;
use type_c_interface::ucsi::Lpm as UcsiLpm;

use super::FnCall as ControllerFnCall;
use super::Mock;

/// Contains a [`UcsiLpm`] function call and its arguments
pub enum FnCall {
    ExecuteLpm(lpm::LocalCommand),
}

impl UcsiLpm for Mock {
    async fn execute_lpm_command(&mut self, command: lpm::LocalCommand) -> Result<Option<lpm::ResponseData>, PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Ucsi(FnCall::ExecuteLpm(command)));
        self.next_result_execute_lpm_command
            .expect("next_result_execute_lpm_command not set")
    }
}
