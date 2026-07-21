//! Electrical disconnect port trait implementation
use core::num::NonZeroU8;

use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::controller::electrical_disconnect::ElectricalDisconnect;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + ElectricalDisconnect>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::electrical_disconnect::ElectricalDisconnect
    for Port<'device, C, Shared, TypeCSender, PowerNotifier, LoopbackSender>
{
    async fn execute_electrical_disconnect(&mut self, reconnect_time_s: Option<NonZeroU8>) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .execute_electrical_disconnect(self.port, reconnect_time_s)
            .await
    }
}
