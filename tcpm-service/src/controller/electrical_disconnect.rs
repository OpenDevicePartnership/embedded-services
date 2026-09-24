//! Electrical disconnect port trait implementation
use core::num::NonZeroU8;

use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use tcpm_interface::controller::electrical_disconnect::ElectricalDisconnect;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + ElectricalDisconnect>,
    Shared: Lockable<Inner = SharedState>,
    PortNotifier: tcpm_interface::port::notification::Notifier,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> tcpm_interface::port::electrical_disconnect::ElectricalDisconnect
    for Port<'device, C, Shared, PortNotifier, PowerNotifier, LoopbackSender>
{
    async fn execute_electrical_disconnect(&mut self, reconnect_time_s: Option<NonZeroU8>) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .execute_electrical_disconnect(self.port, reconnect_time_s)
            .await
    }
}
