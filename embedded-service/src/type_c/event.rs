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
    #[derive(Copy, Clone, PartialEq, Eq, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct PortStatusChangedRaw(u16);
    impl Debug;
    /// Plug inserted or removed
    pub u8, plug_inserted_or_removed, set_plug_inserted_or_removed: 0, 0;
    /// Source Caps received
    pub u8, source_caps_received, set_source_caps_received: 1, 1;
    /// New power contract as provider
    pub u8, new_power_contract_as_provider, set_new_power_contract_as_provider: 2, 2;
    /// New power contract as consumer
    pub u8, new_power_contract_as_consumer, set_new_power_contract_as_consumer: 3, 3;
    /// Sink ready
    pub u8, sink_ready, set_sink_ready: 4, 4;
    /// Power swap completed
    pub u8, power_swap_completed, set_power_swap_completed: 5, 5;
    /// Data swap completed
    pub u8, data_swap_completed, set_data_swap_completed: 6, 6;
    /// Alternate Mode Entered
    pub u8, alt_mode_entered, set_alt_mode_entered: 7, 7;
    /// PD hard reset
    pub u8, pd_hard_reset, set_pd_hard_reset: 8, 8;
}

/// Port status change events
/// This is a type-safe wrapper around the raw bitfield
/// These events are related to the overall port state and typically need to be considered together.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
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

    /// Returns true if a power swap completed event triggered
    pub fn power_swap_completed(self) -> bool {
        self.0.power_swap_completed() != 0
    }

    /// Sets the power swap completed event
    pub fn set_power_swap_completed(&mut self, value: bool) {
        self.0.set_power_swap_completed(value.into());
    }

    /// Returns true if a data swap completed event triggered
    pub fn data_swap_completed(self) -> bool {
        self.0.data_swap_completed() != 0
    }

    /// Sets the data swap completed event
    pub fn set_data_swap_completed(&mut self, value: bool) {
        self.0.set_data_swap_completed(value.into());
    }

    /// Returns true if a alt mode entered event triggered
    pub fn alt_mode_entered(self) -> bool {
        self.0.alt_mode_entered() != 0
    }

    /// Sets the alt mode entered event
    pub fn set_alt_mode_entered(&mut self, value: bool) {
        self.0.set_alt_mode_entered(value.into());
    }

    /// Returns true if a PD hard reset event triggered
    pub fn pd_hard_reset(self) -> bool {
        self.0.pd_hard_reset() != 0
    }

    /// Sets the PD hard reset event
    pub fn set_pd_hard_reset(&mut self, value: bool) {
        self.0.set_pd_hard_reset(value.into());
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
     /// user svid mode entered
    pub u8, custom_mode_entered, set_custom_mode_entered: 1, 1;
    /// user svid mode exited
    pub u8, custom_mode_exited, set_custom_mode_exited: 2, 2;
    /// user svid attention vdm received
    pub u8, custom_mode_attention_received, set_custom_mode_attention_received: 3, 3;
    /// user svid other vdm received
    pub u8, custom_mode_other_vdm_received, set_custom_mode_other_vdm_received: 4, 4;
    /// discover mode completed
    pub u8, discover_mode_completed, set_discover_mode_completed: 5, 5;
    /// usb mux error recovery
    pub u8, usb_mux_error_recovery, set_usb_mux_error_recovery: 6, 6;
    /// DP status update
    pub u8, dp_status_update, set_dp_status_update: 15, 15;
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

    /// Returns true if a custom mode entered event triggered
    pub fn custom_mode_entered(self) -> bool {
        self.0.custom_mode_entered() != 0
    }

    /// Sets the custom mode entered event
    pub fn set_custom_mode_entered(&mut self, value: bool) {
        self.0.set_custom_mode_entered(value.into());
    }

    /// Returns true if a custom mode exited event triggered
    pub fn custom_mode_exited(self) -> bool {
        self.0.custom_mode_exited() != 0
    }

    /// Sets the custom mode exited event
    pub fn set_custom_mode_exited(&mut self, value: bool) {
        self.0.set_custom_mode_exited(value.into());
    }

    /// Returns true if a custom mode attention received event triggered
    pub fn custom_mode_attention_received(self) -> bool {
        self.0.custom_mode_attention_received() != 0
    }

    /// Sets the custom mode attention received event
    pub fn set_custom_mode_attention_received(&mut self, value: bool) {
        self.0.set_custom_mode_attention_received(value.into());
    }

    /// Returns true if a custom mode other VDM received event triggered
    pub fn custom_mode_other_vdm_received(self) -> bool {
        self.0.custom_mode_other_vdm_received() != 0
    }

    /// Sets the custom mode other VDM received event
    pub fn set_custom_mode_other_vdm_received(&mut self, value: bool) {
        self.0.set_custom_mode_other_vdm_received(value.into());
    }

    /// Returns true if a discover mode completed event triggered
    pub fn discover_mode_completed(self) -> bool {
        self.0.discover_mode_completed() != 0
    }

    /// Sets the discover mode completed event
    pub fn set_discover_mode_completed(&mut self, value: bool) {
        self.0.set_discover_mode_completed(value.into());
    }

    /// Returns true if a USB mux error recovery event triggered
    pub fn usb_mux_error_recovery(self) -> bool {
        self.0.usb_mux_error_recovery() != 0
    }

    /// Sets the USB mux error recovery event
    pub fn set_usb_mux_error_recovery(&mut self, value: bool) {
        self.0.set_usb_mux_error_recovery(value.into());
    }

    /// Returns true if a DP status update event triggered
    pub fn dp_status_update(self) -> bool {
        self.0.dp_status_update() != 0
    }

    /// Sets the DP status update event
    pub fn set_dp_status_update(&mut self, value: bool) {
        self.0.set_dp_status_update(value.into());
    }
}

/// Individual port notifications
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortNotificationSingle {
    /// PD alert
    Alert,
    /// Custom mode entered
    CustomModeEntered,
    /// Custom mode exited
    CustomModeExited,
    /// Custom mode attention received
    CustomModeAttentionReceived,
    /// Custom mode other VDM received
    CustomModeOtherVdmReceived,
    /// Discover mode completed
    DiscoverModeCompleted,
    /// USB mux error recovery
    UsbMuxErrorRecovery,
    /// DP status update
    DpStatusUpdate,
}

impl Iterator for PortNotification {
    type Item = PortNotificationSingle;

    fn next(&mut self) -> Option<Self::Item> {
        if self.alert() {
            self.set_alert(false);
            Some(PortNotificationSingle::Alert)
        } else if self.custom_mode_entered() {
            self.set_custom_mode_entered(false);
            Some(PortNotificationSingle::CustomModeEntered)
        } else if self.custom_mode_exited() {
            self.set_custom_mode_exited(false);
            Some(PortNotificationSingle::CustomModeExited)
        } else if self.custom_mode_attention_received() {
            self.set_custom_mode_attention_received(false);
            Some(PortNotificationSingle::CustomModeAttentionReceived)
        } else if self.custom_mode_other_vdm_received() {
            self.set_custom_mode_other_vdm_received(false);
            Some(PortNotificationSingle::CustomModeOtherVdmReceived)
        } else if self.discover_mode_completed() {
            self.set_discover_mode_completed(false);
            Some(PortNotificationSingle::DiscoverModeCompleted)
        } else if self.usb_mux_error_recovery() {
            self.set_usb_mux_error_recovery(false);
            Some(PortNotificationSingle::UsbMuxErrorRecovery)
        } else if self.dp_status_update() {
            self.set_dp_status_update(false);
            Some(PortNotificationSingle::DpStatusUpdate)
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

impl Default for PortEvent {
    fn default() -> Self {
        Self::none()
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

impl FromIterator<usize> for PortPending {
    fn from_iter<T: IntoIterator<Item = usize>>(iter: T) -> Self {
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
        notification.set_custom_mode_entered(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::Alert));
        assert_eq!(notification.next(), Some(PortNotificationSingle::CustomModeEntered));
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
    fn test_port_notification_iter_custom_mode_entered() {
        let mut notification = PortNotification::none();
        notification.set_custom_mode_entered(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::CustomModeEntered));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_custom_mode_exited() {
        let mut notification = PortNotification::none();
        notification.set_custom_mode_exited(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::CustomModeExited));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_custom_mode_attention_received() {
        let mut notification = PortNotification::none();
        notification.set_custom_mode_attention_received(true);

        assert_eq!(
            notification.next(),
            Some(PortNotificationSingle::CustomModeAttentionReceived)
        );
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_custom_mode_other_vdm_received() {
        let mut notification = PortNotification::none();
        notification.set_custom_mode_other_vdm_received(true);

        assert_eq!(
            notification.next(),
            Some(PortNotificationSingle::CustomModeOtherVdmReceived)
        );
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_discover_mode_completed() {
        let mut notification = PortNotification::none();
        notification.set_discover_mode_completed(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::DiscoverModeCompleted));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_usb_mux_error_recovery() {
        let mut notification = PortNotification::none();
        notification.set_usb_mux_error_recovery(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::UsbMuxErrorRecovery));
        assert_eq!(notification.next(), None);
    }

    #[test]
    fn test_port_notification_iter_dp_status_update() {
        let mut notification = PortNotification::none();
        notification.set_dp_status_update(true);

        assert_eq!(notification.next(), Some(PortNotificationSingle::DpStatusUpdate));
        assert_eq!(notification.next(), None);
    }
}
