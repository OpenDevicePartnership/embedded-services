//! Events originating from a charger device

use embedded_services::sync::Lockable;

/// PSU state as determined by charger device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PsuState {
    /// Charger detected PSU attached
    Attached,
    /// Charger detected PSU detached
    Detached,
}

impl From<bool> for PsuState {
    fn from(value: bool) -> Self {
        match value {
            true => PsuState::Attached,
            false => PsuState::Detached,
        }
    }
}

/// Data for a charger event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EventData {
    /// PSU state changed
    PsuStateChange(PsuState),
}

/// Event broadcast from a charger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Event<'a, D: Lockable>
where
    D::Inner: crate::charger::Charger,
{
    /// Device that sent this request
    pub charger: &'a D,
    /// Event data
    pub event: EventData,
}
