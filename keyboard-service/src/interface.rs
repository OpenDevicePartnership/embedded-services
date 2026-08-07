//! Simple keyboard service interface with reports that are compatible with the HID boot protocol.

use embassy_sync::pubsub::{DynSubscriber, Error};

/// A standard 8-byte boot-protocol keyboard input report
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct KeyboardInputReport(pub(crate) [u8; 8]);

impl KeyboardInputReport {
    /// Creates an input report from its modifier byte and key usage codes.
    pub const fn new(modifiers: u8, keys: [u8; 6]) -> Self {
        Self([modifiers, 0, keys[0], keys[1], keys[2], keys[3], keys[4], keys[5]])
    }

    /// Returns the serialized report payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn error(code: u8) -> Self {
        let mut report = Self::default();
        for slot in report.0.iter_mut().skip(2) {
            *slot = code;
        }
        report
    }
}

impl From<keyberon::key_code::KbHidReport> for KeyboardInputReport {
    fn from(report: keyberon::key_code::KbHidReport) -> Self {
        const _: () = {
            assert!(
                core::mem::size_of::<keyberon::key_code::KbHidReport>() == core::mem::size_of::<KeyboardInputReport>(),
                "keyberon::key_code::KbHidReport and KeyboardInputReport must be the same size/layout"
            );
        };

        // Panic safety: we statically assert that the sizes match above, so the build will fail if it's possible for this conversion to panic.
        #[allow(clippy::expect_used)]
        Self(
            report
                .as_bytes()
                .try_into()
                .expect("KbHidReport is exactly 8 bytes, which we statically asserted above"),
        )
    }
}

/// Keyboard power state requested by a relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardPowerState {
    /// Scan for and publish input reports.
    On,
    /// Stop scanning and publishing input reports.
    Sleep,
    /// Stop scanning and do not wake the host.
    Off,
}

bitflags::bitflags! {
    /// Keyboard LED states controlled by the host.
    pub struct LedFlags: u8 {
        const NumLock = 1 << 0;
        const CapsLock = 1 << 1;
        const ScrollLock = 1 << 2;
        const _ = !0;
    }
}

/// Interface consumed by keyboard relay handlers.
///
/// Implementations own keyboard-specific scanning and hardware control. Relay handlers translate
/// between this interface and a protocol such as HID.
pub trait KeyboardService<'hw> {
    /// Error returned when applying a host-controlled keyboard setting.
    type Error;

    /// Applies the requested keyboard LED state.
    fn set_leds(&mut self, flags: LedFlags) -> impl core::future::Future<Output = Result<(), Self::Error>>;

    /// Applies the host-commanded power state.
    fn set_power_state(&mut self, state: KeyboardPowerState);

    /// Subscribes to input reports produced by the keyboard.
    fn subscriber(&self) -> Result<DynSubscriber<'hw, KeyboardInputReport>, Error>;
}
