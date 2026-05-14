//! Max sink voltage port trait implementation
use embedded_services::{event::Sender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::controller::max_sink_voltage::MaxSinkVoltage;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + MaxSinkVoltage>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
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
