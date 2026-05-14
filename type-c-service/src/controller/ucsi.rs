//! UCSI LPM port trait implementation
use embedded_services::{event::Sender, sync::Lockable};
use embedded_usb_pd::{PdError, ucsi::lpm};
use type_c_interface::ucsi::Lpm as UcsiLpm;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + UcsiLpm>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> type_c_interface::ucsi::Lpm for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn execute_lpm_command(&mut self, command: lpm::LocalCommand) -> Result<Option<lpm::ResponseData>, PdError> {
        self.controller.lock().await.execute_lpm_command(command).await
    }
}
