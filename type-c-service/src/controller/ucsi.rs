//! UCSI LPM port trait implementation
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::{PdError, ucsi::v1_2::lpm};
use type_c_interface::ucsi::Lpm as UcsiLpm;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + UcsiLpm>,
    Shared: Lockable<Inner = SharedState>,
    PortNotifier: type_c_interface::port::notification::Notifier,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::ucsi::Lpm for Port<'device, C, Shared, PortNotifier, PowerNotifier, LoopbackSender>
{
    async fn execute_lpm_command(&mut self, command: lpm::LocalCommand) -> Result<Option<lpm::ResponseData>, PdError> {
        self.controller.lock().await.execute_lpm_command(command).await
    }
}
