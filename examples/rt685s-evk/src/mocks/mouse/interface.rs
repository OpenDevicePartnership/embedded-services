//! A simplified interface for a hypothetical mouse service.
pub trait MouseService<'hw> {
    fn subscriber(
        &self,
    ) -> Result<embassy_sync::pubsub::DynSubscriber<'hw, MouseInputReport>, embassy_sync::pubsub::Error>;
}

/// A report from the mouse.  Note that mice are very HID-centric so it makes sense that this has the same
/// underlying representation as a HID mouse input report, but this is an optimization that may not make sense for
/// other service types (e.g. time-alarm may just emit a non-repr(C) enum of the kind of event and leave it up to
/// the relay handler to figure out how to translate that to HID (or MCTP or some other logical protocol).
///
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
pub struct MouseInputReport {
    /// 3 bits used for buttons, 5 bits padding
    pub buttons: u8,

    /// Relative X movement
    pub x: i8,

    /// Relative Y movement
    pub y: i8,
}
