//! Electrical disconnect port trait implementation
use core::num::NonZeroU8;

use embedded_services::{event::Sender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::controller::electrical_disconnect::ElectricalDisconnect;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + ElectricalDisconnect>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> type_c_interface::port::electrical_disconnect::ElectricalDisconnect
    for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn execute_electrical_disconnect(&mut self, reconnect_time_s: Option<NonZeroU8>) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .execute_electrical_disconnect(self.port, reconnect_time_s)
            .await
    }
}
