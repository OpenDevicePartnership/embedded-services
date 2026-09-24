use embedded_usb_pd::{LocalPortId, PdError};

use crate::{control::type_c::TypeCStateMachineState, controller::pd::Pd};

/// Type-C state machine related controller functionality
pub trait StateMachine: Pd {
    /// Set Type-C state-machine configuration for the given port
    fn set_type_c_state_machine_config(
        &mut self,
        port: LocalPortId,
        state: TypeCStateMachineState,
    ) -> impl Future<Output = Result<(), PdError>>;
}
