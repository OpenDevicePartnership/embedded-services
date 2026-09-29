//! General power related control types

/// System power state
///
/// Used to notify the PD controller of the current system power state,
/// which triggers Application Configuration updates (e.g., crossbar reconfiguration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SystemPowerState {
    /// S0 - System fully running
    S0,
    /// S3 - Suspend to RAM
    S3,
    /// S4 - Hibernate
    S4,
    /// S5 - Soft off
    S5,
    /// S0ix - Modern standby / Connected standby
    S0ix,
}
