//! Max sink voltage port trait implementation
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::controller::max_sink_voltage::MaxSinkVoltage;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + MaxSinkVoltage>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::max_sink_voltage::MaxSinkVoltage
    for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn set_max_sink_voltage(&mut self, voltage_mv: Option<u16>) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_max_sink_voltage(self.port, voltage_mv)
            .await
    }
}
