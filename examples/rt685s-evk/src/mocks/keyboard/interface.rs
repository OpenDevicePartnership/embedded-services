///! A simplified interface for a hypothetical keyboard service.
pub trait KeyboardService<'hw> {
    fn set_led(&mut self, state: u8) -> impl core::future::Future<Output = ()> + Send;

    fn subscriber(
        &self,
    ) -> Result<embassy_sync::pubsub::DynSubscriber<'hw, KeyboardInputReport>, embassy_sync::pubsub::Error>;
}

/// A report from the keyboard.  Note that keyboards are very HID-centric so it makes sense that this has the same
/// underlying representation as a HID keyboard input report, but this is an optimization that may not make sense for
/// other service types (e.g. time-alarm may just emit a non-repr(C) enum of the kind of event and leave it up to
/// the relay handler to figure out how to translate that to HID (or MCTP or some other logical protocol).
///
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
pub struct KeyboardInputReport {
    /// Left Ctrl .. Right GUI
    pub modifiers: u8,

    /// Reserved byte required by boot keyboard format
    pub reserved: u8,

    /// Up to 6 simultaneous key usages
    pub keys: [u8; 6],
}

/// A (very) simplified list of the possible keys that can be pressed on a keyboard.
#[repr(u8)]
#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, Clone, Copy, defmt::Format)]
pub enum KeyCode {
    NumLock = 0x53,
    A = 0x04,
}

#[repr(u8)]
pub enum KeyboardLedFlags {
    NumLock = 0x01,
    CapsLock = 0x02,
    ScrollLock = 0x04,
}
