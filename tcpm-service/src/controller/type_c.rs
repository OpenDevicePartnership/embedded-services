//! Type-C state machine port trait implementation
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use tcpm_interface::control::type_c::TypeCStateMachineState;
use tcpm_interface::controller::type_c::StateMachine;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + StateMachine>,
    Shared: Lockable<Inner = SharedState>,
    PortNotifier: tcpm_interface::port::notification::Notifier,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> tcpm_interface::port::type_c::StateMachine
    for Port<'device, C, Shared, PortNotifier, PowerNotifier, LoopbackSender>
{
    async fn set_type_c_state_machine_config(&mut self, state: TypeCStateMachineState) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_type_c_state_machine_config(self.port, state)
            .await
    }
}
