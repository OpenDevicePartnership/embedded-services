//! Type-C state machine port trait implementation
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::control::type_c::TypeCStateMachineState;
use type_c_interface::controller::type_c::StateMachine;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + StateMachine>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::type_c::StateMachine for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn set_type_c_state_machine_config(&mut self, state: TypeCStateMachineState) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_type_c_state_machine_config(self.port, state)
            .await
    }
}
