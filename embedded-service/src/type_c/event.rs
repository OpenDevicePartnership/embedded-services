//! This module provides TCPM event types and bitfields.
//! Hardware typically uses bitfields to store pending events/interrupts so we provide generic versions of these.
//! [`PortStatusChanged`] contains events related to the overall port state (plug state, power contract, etc).
//! Processing these events typically requires acessing similar registers so they are grouped together.
//! [`PortNotification`] contains events that are typically more message-like (PD alerts, VDMs, etc) and can be processed independently.
//! Consequently [`PortNotification`] implements iterator traits to allow for processing these events as a stream.
use bitfield::bitfield;
use bitvec::BitArr;

bitfield! {
    /// Raw bitfield of possible port status events
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct PortStatusChangedRaw(u16);
    impl Debug;
    /// Plug inserted or removed
    pub u8, plug_inserted_or_removed, set_plug_inserted_or_removed: 0, 0;
    /// New power contract as provider
    pub u8, new_power_contract_as_provider, set_new_power_contract_as_provider: 2, 2;
    /// New power contract as consumer
    pub u8, new_power_contract_as_consumer, set_new_power_contract_as_consumer: 3, 3;
    /// Source Caps received
    pub u8, source_caps_received, set_source_caps_received: 4, 4;
    /// Sink ready
    pub u8, sink_ready, set_sink_ready: 5, 5;
}

/// Port status change events
/// This is a type-safe wrapper around the raw bitfield
/// These events are related to the overall port state and typically need to be considered together.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortStatusChanged(PortStatusChangedRaw);

impl PortStatusChanged {
    /// Create a new PortEventKind with no pending events
    pub const fn none() -> Self {
        Self(PortStatusChangedRaw(0))
    }

    /// Returns the union of self and other
    pub fn union(self, other: PortStatusChanged) -> PortStatusChanged {
        // This spacing is what rustfmt wants
        PortStatusChanged(PortStatusChangedRaw(self.0.0 | other.0.0))
    }

    /// Returns true if a plug was inserted or removed
    pub fn plug_inserted_or_removed(self) -> bool {
        self.0.plug_inserted_or_removed() != 0
    }

    /// Sets the plug inserted or removed event
    pub fn set_plug_inserted_or_removed(&mut self, value: bool) {
        self.0.set_plug_inserted_or_removed(value.into());
    }

    /// Returns true if a new power contract was established as provider
    pub fn new_power_contract_as_provider(&self) -> bool {
        self.0.new_power_contract_as_provider() != 0
    }

    /// Sets the new power contract as provider event
    pub fn set_new_power_contract_as_provider(&mut self, value: bool) {
        self.0.set_new_power_contract_as_provider(value.into());
    }

    /// Returns true if a new power contract was established as consumer
    pub fn new_power_contract_as_consumer(self) -> bool {
        self.0.new_power_contract_as_consumer() != 0
    }

    /// Sets the new power contract as consumer event
    pub fn set_new_power_contract_as_consumer(&mut self, value: bool) {
        self.0.set_new_power_contract_as_consumer(value.into());
    }

    /// Returns true if a source caps msg received
    pub fn source_caps_received(self) -> bool {
        self.0.source_caps_received() != 0
    }

    /// Sets the source caps received event
    pub fn set_source_caps_received(&mut self, value: bool) {
        self.0.set_source_caps_received(value.into());
    }

    /// Returns true if a sink ready event triggered
    pub fn sink_ready(self) -> bool {
        self.0.sink_ready() != 0
    }

    /// Sets the sink ready event
    pub fn set_sink_ready(&mut self, value: bool) {
        self.0.set_sink_ready(value.into());
    }
}

bitfield! {
    /// Raw bitfield of possible port notification events
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct PortNotificationRaw(u16);
    impl Debug;
    /// PD alert
    pub u8, alert, set_alert: 0, 0;
    /// Received a VDM
    pub u8, vdm, set_vdm: 1, 1;
}

/// Port notification events
/// This is a type-safe wrapper around the raw bitfield
/// These events are unrelated to the overall port state and each other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortNotification(PortNotificationRaw);

impl PortNotification {
    /// Create a new PortNotification with no pending events
    pub const fn none() -> Self {
        Self(PortNotificationRaw(0))
    }

    /// Returns the union of self and other
    pub fn union(self, other: PortNotification) -> PortNotification {
        // This spacing is what rustfmt wants
        PortNotification(PortNotificationRaw(self.0.0 | other.0.0))
    }

    /// Returns true if an alert was received
    pub fn alert(self) -> bool {
        self.0.alert() != 0
    }

    /// Sets the alert event
    pub fn set_alert(&mut self, value: bool) {
        self.0.set_alert(value.into());
    }

    /// Returns true if a VDM was received
    pub fn vdm(self) -> bool {
        self.0.vdm() != 0
    }

    /// Sets the VDM event
    pub fn set_vdm(&mut self, value: bool) {
        self.0.set_vdm(value.into());
    }
}

/// Individual port notifications
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortNotificationSingle {
    /// PD alert
    Alert,
    /// Received a VDM
    Vdm,
}

impl Iterator for PortNotification {
    type Item = PortNotificationSingle;

    fn next(&mut self) -> Option<Self::Item> {
        if self.alert() {
            self.set_alert(false);
            Some(PortNotificationSingle::Alert)
        } else if self.vdm() {
            self.set_vdm(false);
            Some(PortNotificationSingle::Vdm)
        } else {
            None
        }
    }
}

/// Overall port event type
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortEvent {
    /// Port status change events
    pub status: PortStatusChanged,
    /// Port notification events
    pub notification: PortNotification,
}

impl PortEvent {
    /// Creates a new PortEvent with no pending events
    pub const fn none() -> Self {
        Self {
            status: PortStatusChanged::none(),
            notification: PortNotification::none(),
        }
    }

    /// Returns the union of self and other
    pub fn union(self, other: PortEvent) -> PortEvent {
        PortEvent {
            status: self.status.union(other.status),
            notification: self.notification.union(other.notification),
        }
    }
}

impl From<PortStatusChanged> for PortEvent {
    fn from(status: PortStatusChanged) -> Self {
        Self {
            status,
            notification: PortNotification::none(),
        }
    }
}

impl From<PortNotification> for PortEvent {
    fn from(notification: PortNotification) -> Self {
        Self {
            status: PortStatusChanged::none(),
            notification,
        }
    }
}

/// Bit vector type to store pending port events
type PortPendingVec = BitArr!(for 32, in u32);

/// Pending port events
///
/// This type works using usize to allow use with both global and local port IDs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct PortPending(PortPendingVec);

impl PortPending {
    /// Creates a new PortPending with no pending ports
    pub const fn none() -> Self {
        Self(PortPendingVec::ZERO)
    }

    /// Returns true if there are no pending ports
    pub fn is_none(&self) -> bool {
        self.0 == PortPendingVec::ZERO
    }

    /// Marks the given port as pending
    pub fn pend_port(&mut self, port: usize) {
        self.0.set(port, true);
    }

    /// Marks the indexes given by the iterator as pending
    pub fn pend_ports<I: IntoIterator<Item = usize>>(&mut self, iter: I) {
        for port in iter {
            self.pend_port(port);
        }
    }

    /// Clears the pending status of the given port
    pub fn clear_port(&mut self, port: usize) {
        self.0.set(port, false);
    }

    /// Returns true if the given port is pending
    pub fn is_pending(&self, port: usize) -> bool {
        self.0[port]
    }

    /// Returns a combination of the current pending ports and other
    pub fn union(&self, other: PortPending) -> PortPending {
        PortPending(self.0 | other.0)
    }

    /// Returns the number of bits in Self
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<PortPending> for u32 {
    fn from(flags: PortPending) -> Self {
        flags.0.data[0]
    }
}

impl<I: Iterator<Item = usize>> From<I> for PortPending {
    fn from(iter: I) -> Self {
        let mut flags = PortPending::none();
        flags.pend_ports(iter);
        flags
    }
}

/// An iterator over the pending port event flags
#[derive(Copy, Clone)]
pub struct PortPendingIter {
    /// The flags being iterated over
    flags: PortPending,
    /// The current index in the flags
    index: usize,
}

impl Iterator for PortPendingIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.flags.len() {
            let port_index = self.index;
            self.index += 1;
            if self.flags.is_pending(port_index) {
                self.flags.clear_port(port_index);
                return Some(port_index);
            }
        }
        None
    }
}

impl IntoIterator for PortPending {
    type Item = usize;
    type IntoIter = PortPendingIter;

    fn into_iter(self) -> PortPendingIter {
        PortPendingIter { flags: self, index: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_event_flags_iter() {
        let mut pending = PortPending::none();

        pending.pend_port(0);
        pending.pend_port(1);
        pending.pend_port(2);
        pending.pend_port(10);
        pending.pend_port(23);
        pending.pend_port(31);

        let mut iter = pending.into_iter();
        assert_eq!(iter.next(), Some(0));
        assert_eq!(iter.next(), Some(1));
        assert_eq!(iter.next(), Some(2));
        assert_eq!(iter.next(), Some(10));
        assert_eq!(iter.next(), Some(23));
        assert_eq!(iter.next(), Some(31));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_port_notification_iter_all() {
        let mut notification = PortNotification::none();
        notification.set_alert(true);
        notification.set_vdm(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::Alert));
        assert_eq!(notification.next(), Some(PortNotificationSingle::Vdm));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_alert() {
        let mut notification = PortNotification::none();
        notification.set_alert(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::Alert));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_vdm() {
        let mut notification = PortNotification::none();
        notification.set_vdm(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::Vdm));
        assert_eq!(notification.next(), None);
    }
}
