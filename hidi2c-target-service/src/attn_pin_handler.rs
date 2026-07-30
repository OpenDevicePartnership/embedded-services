use crate::*;

/// Handler for the ATTN pin, which is used to signal the host that we have an input report ready to be read.
/// This is a simple wrapper around an OutputPin that tracks whether we've asserted the interrupt or not, because
/// OutputPin doesn't have a built-in way to interrogate its own state.  There is a StatefulOutputPin trait that
/// does have that functionality, but not all pins support it so we just implement it ourselves here.
///
pub(crate) struct AttnPinHandler<AttnPin: embedded_hal::digital::OutputPin> {
    attn_pin: AttnPin,
    asserted: bool,
}

impl<AttnPin: embedded_hal::digital::OutputPin> AttnPinHandler<AttnPin> {
    /// Construct a new handler that owns the provided GPIO hardware
    pub(crate) fn new(attn_pin: AttnPin) -> Self {
        let mut result = Self {
            attn_pin,
            asserted: false,
        };
        let _ = result.clear_interrupt();
        result
    }

    /// Clear the interrupt, which is done by setting the pin high.
    pub(crate) fn clear_interrupt(&mut self) -> Result<(), AttnPin::Error> {
        trace!("HID-I2C: ATTN: clear interrupt");
        self.attn_pin.set_high()?;
        self.asserted = false;
        Ok(())
    }

    /// Assert the interrupt, which is done by pulling the pin low.
    pub(crate) fn assert_interrupt(&mut self) -> Result<(), AttnPin::Error> {
        trace!("HID-I2C: ATTN: assert interrupt");
        self.attn_pin.set_low()?;
        self.asserted = true;
        Ok(())
    }

    /// Returns true if we are asserting the interrupt, false otherwise.
    pub(crate) fn asserted(&self) -> bool {
        self.asserted
    }
}
