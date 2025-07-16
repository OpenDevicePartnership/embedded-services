//! Event definitions
use bitfield::bitfield;
use bitvec::BitArr;
use embedded_usb_pd::GlobalPortId;

bitfield! {
    /// Raw bitfield of possible port events
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct PortEventKindRaw(u32);
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
    /// usb mux error recovery
    pub u8, usb_mux_error_recovery, set_usb_mux_error_recovery: 9, 9;
    /// user svid mode entered
    pub u8, custom_mode_entered, set_custom_mode_entered: 10, 10;
    /// user svid mode exited
    pub u8, custom_mode_exited, set_custom_mode_exited: 11, 11;
    /// user svid attention vdm received
    pub u8, custom_mode_attention_received, set_custom_mode_attention_received: 12, 12;
    /// user svid other vdm received
    pub u8, custom_mode_other_vdm_received, set_custom_mode_other_vdm_received: 13, 13;
    /// discover mode completed
    pub u8, discover_mode_completed, set_discover_mode_completed: 14, 14;
    /// DP status update
    pub u8, dp_status_update, set_dp_status_update: 15, 15;
    /// PD Alert received
    pub u8, pd_alert_received, set_pd_alert_received: 16, 16;
}

/// Type-safe wrapper for the raw port event kind
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortEventKind(PortEventKindRaw);

impl PortEventKind {
    /// Create a new PortEventKind with no pending events
    pub const fn none() -> Self {
        Self(PortEventKindRaw(0))
    }

    /// Returns the union of self and other
    pub fn union(self, other: PortEventKind) -> PortEventKind {
        // This spacing is what rustfmt wants
        PortEventKind(PortEventKindRaw(self.0.0 | other.0.0))
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

    /// Returns true if a USB mux error recovery event triggered
    pub fn usb_mux_error_recovery(self) -> bool {
        self.0.usb_mux_error_recovery() != 0
    }

    /// Sets the USB mux error recovery event
    pub fn set_usb_mux_error_recovery(&mut self, value: bool) {
        self.0.set_usb_mux_error_recovery(value.into());
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

    /// Returns true if a DP status update event triggered
    pub fn dp_status_update(self) -> bool {
        self.0.dp_status_update() != 0
    }

    /// Sets the DP status update event
    pub fn set_dp_status_update(&mut self, value: bool) {
        self.0.set_dp_status_update(value.into());
    }

    /// Returns true if a PD alert received event triggered
    pub fn pd_alert_received(self) -> bool {
        self.0.pd_alert_received() != 0
    }

    /// Sets the PD alert received event
    pub fn set_pd_alert_received(&mut self, value: bool) {
        self.0.set_pd_alert_received(value.into());
    }
}

/// Bit vector type to store pending port events
type PortEventFlagsVec = BitArr!(for 32, in u32);

/// Pending port events
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct PortEventFlags(PortEventFlagsVec);

impl PortEventFlags {
    /// Creates a new PortEventFlags with no pending events
    pub const fn none() -> Self {
        Self(PortEventFlagsVec::ZERO)
    }

    /// Returns true if there are no pending events
    pub fn is_none(&self) -> bool {
        self.0 == PortEventFlagsVec::ZERO
    }

    /// Marks the given port as pending
    pub fn pend_port(&mut self, port: GlobalPortId) {
        self.0.set(port.0 as usize, true);
    }

    /// Clears the pending status of the given port
    pub fn clear_port(&mut self, port: GlobalPortId) {
        self.0.set(port.0 as usize, false);
    }

    /// Returns true if the given port is pending
    pub fn is_pending(&self, port: GlobalPortId) -> bool {
        self.0[port.0 as usize]
    }

    /// Returns a combination of the current event flags and other
    pub fn union(&self, other: PortEventFlags) -> PortEventFlags {
        PortEventFlags(self.0 | other.0)
    }

    /// Returns the number of bits in the event
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<PortEventFlags> for u32 {
    fn from(flags: PortEventFlags) -> Self {
        flags.0.data[0]
    }
}

/// An iterator over the pending port event flags
pub struct PortEventFlagsIter {
    /// The flags being iterated over
    flags: PortEventFlags,
    /// The current index in the flags
    index: usize,
}

impl Iterator for PortEventFlagsIter {
    type Item = GlobalPortId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.flags.len() {
            let port_id = GlobalPortId(self.index as u8);
            if self.flags.is_pending(port_id) {
                self.index += 1;
                return Some(port_id);
            }
            self.index += 1;
        }
        None
    }
}

impl IntoIterator for PortEventFlags {
    type Item = GlobalPortId;
    type IntoIter = PortEventFlagsIter;

    fn into_iter(self) -> PortEventFlagsIter {
        PortEventFlagsIter { flags: self, index: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_event_flags_iter() {
        let mut pending = PortEventFlags::none();

        pending.pend_port(GlobalPortId(0));
        pending.pend_port(GlobalPortId(1));
        pending.pend_port(GlobalPortId(2));
        pending.pend_port(GlobalPortId(10));
        pending.pend_port(GlobalPortId(23));
        pending.pend_port(GlobalPortId(31));

        let mut iter = pending.into_iter();
        assert_eq!(iter.next(), Some(GlobalPortId(0)));
        assert_eq!(iter.next(), Some(GlobalPortId(1)));
        assert_eq!(iter.next(), Some(GlobalPortId(2)));
        assert_eq!(iter.next(), Some(GlobalPortId(10)));
        assert_eq!(iter.next(), Some(GlobalPortId(23)));
        assert_eq!(iter.next(), Some(GlobalPortId(31)));
        assert_eq!(iter.next(), None);
    }
}
