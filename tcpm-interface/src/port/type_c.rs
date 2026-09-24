use embedded_usb_pd::PdError;

use crate::{control::type_c::TypeCStateMachineState, port::pd::Pd};

/// Type-C state machine related controller functionality
pub trait StateMachine: Pd {
    /// Set Type-C state-machine configuration for this port
    fn set_type_c_state_machine_config(
        &mut self,
        state: TypeCStateMachineState,
    ) -> impl Future<Output = Result<(), PdError>>;
}
