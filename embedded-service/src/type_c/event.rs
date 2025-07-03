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
    /// New power contract as provider
    pub u8, new_power_contract_as_provider, set_new_power_contract_as_provider: 2, 2;
    /// New power contract as consumer
    pub u8, new_power_contract_as_consumer, set_new_power_contract_as_consumer: 3, 3;
    /// Source Caps received
    pub u8, source_caps_received, set_source_caps_received: 4, 4;
    /// Sink ready
    pub u8, sink_ready, set_sink_ready: 5, 5;
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
