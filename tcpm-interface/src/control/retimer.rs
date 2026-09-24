//! Retimer related control types

/// Retimer update state
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RetimerFwUpdateState {
    /// Retimer FW Update Inactive
    Inactive,
    /// Retimer FW Update Active
    Active,
}
