use crate::*;

/// Handler for the ATTN pin, which is used to signal the host that we have an input report ready to be read.
/// This is a simple wrapper around an OutputPin that tracks whether we've asserted the interrupt or not, because
/// OutputPin doesn't have a built-in way to interrogate its own state.
///
pub struct AttnPinHandler<AttnPin: embedded_hal::digital::OutputPin> {
    attn_pin: AttnPin,
    asserted: bool,
}

impl<AttnPin: embedded_hal::digital::OutputPin> AttnPinHandler<AttnPin> {
    /// Construct a new handler that owns the provided GPIO hardware
    pub fn new(attn_pin: AttnPin) -> Self {
        let mut result = Self {
            attn_pin,
            asserted: false,
        };
        result.clear_interrupt();
        result
    }

    /// Clear the interrupt, which is done by setting the pin high.
    pub fn clear_interrupt(&mut self) {
        trace!("HID-I2C: ATTN: clear interrupt");
        self.attn_pin
            .set_high()
            .unwrap_or_else(|_| error!("HID-I2C: Failed to clear interrupt on attn pin"));
        self.asserted = false;
    }

    /// Assert the interrupt, which is done by pulling the pin low.
    pub fn assert_interrupt(&mut self) {
        trace!("HID-I2C: ATTN: assert interrupt");
        self.attn_pin
            .set_low()
            .unwrap_or_else(|_| error!("HID-I2C: Failed to assert interrupt on attn pin"));
        self.asserted = true;
    }

    /// Returns true if we are asserting the interrupt, false otherwise.
    pub fn asserted(&self) -> bool {
        self.asserted
    }
}
