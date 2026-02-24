pub mod event;

/// Unconstrained state information
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UnconstrainedState {
    /// Unconstrained state
    pub unconstrained: bool,
    /// Available unconstrained devices
    pub available: usize,
}

impl UnconstrainedState {
    /// Create a new unconstrained state
    pub fn new(unconstrained: bool, available: usize) -> Self {
        Self {
            unconstrained,
            available,
        }
    }
}
