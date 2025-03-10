//! Power policy actions
//! This modules contains wrapper structs that use type states to enforce the valid actions for each device state
pub mod device;

use super::component::ComponentState;

trait Sealed {}

/// Trait to provide the kind of a state type
#[allow(private_bounds)]
pub trait Kind: Sealed {
    /// Return the kind of a state type
    fn kind() -> ComponentState;
}

/// State type for an idle component
pub struct Idle;
impl Sealed for Idle {}
impl Kind for Idle {
    fn kind() -> ComponentState {
        ComponentState::Idle
    }
}

/// State type for an idle component
pub struct Ready;
impl Sealed for Ready {}
impl Kind for Ready {
    fn kind() -> ComponentState {
        ComponentState::Ready
    }
}

/// State type for an busy component
pub struct Busy;
impl Sealed for Busy {}
impl Kind for Busy {
    fn kind() -> ComponentState {
        ComponentState::Busy
    }
}

/// State type for an component that's finalizing an update
pub struct FinalizingUpdate;
impl Sealed for FinalizingUpdate {}
impl Kind for FinalizingUpdate {
    fn kind() -> ComponentState {
        ComponentState::FinalizingUpdate
    }
}
