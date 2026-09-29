//! Type-C related control types

/// TypeC State Machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TypeCStateMachineState {
    /// Sink state machine only
    Sink,
    /// Source state machine only
    Source,
    /// DRP state machine
    Drp,
    /// Disabled
    Disabled,
}
