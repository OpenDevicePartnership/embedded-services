//! Port event types

use type_c_interface::port::event::PortEventBitfield;

/// Top-level port event type
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event {
    /// Port event
    PortEvent(type_c_interface::port::event::PortEvent),
}

/// Loopback event to allow `sync_state` and similar functions
/// to generate events that can be processed by the same code as real events.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Loopback {
    /// Port event
    PortEvent(PortEventBitfield),
    /// Sink ready deadline invalidated
    SinkReadyDeadlineInvalidated,
}
