//! Debounce Module

use embassy_time::{Duration, Timer};
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;

#[derive(Debug)]
/// Enum representing if the button is active low or active high.
pub enum ActiveState {
    /// Button is active low.
    ActiveLow,
    /// Button is active high.
    ActiveHigh,
}

#[derive(Debug)]
/// Struct representing a debouncer for a button.
pub struct Debouncer {
    integrator: u8,
    threshold: u8,
    sample_interval: Duration,
    active_state: ActiveState,
    pressed: bool,
}

impl Debouncer {
    /// Creates a new Debouncer instance with the given threshold value, sampling interval and active state.
    pub fn new(threshold: u8, sample_interval: Duration, active_state: ActiveState) -> Self {
        Self {
            integrator: 0,
            threshold,
            sample_interval,
            active_state,
            pressed: false,
        }
    }

    /// Debounces a button press using an integrator.
    pub async fn debounce<I: InputPin + Wait>(&mut self, gpio: &mut I) -> bool {
        loop {
            // Sample the button state
            let is_pressed = match self.active_state {
                ActiveState::ActiveLow => gpio.is_low().unwrap_or(false),
                ActiveState::ActiveHigh => gpio.is_high().unwrap_or(false),
            };

            // Check if the button is pressed and increment the integrator
            if is_pressed {
                if self.integrator < self.threshold {
                    self.integrator += 1;
                }
            } else if self.integrator > 0 {
                self.integrator -= 1;
            }

            // Check if the integrator has crossed the threshold and the button state has changed
            if self.integrator >= self.threshold && !self.pressed {
                self.pressed = true;
                return true;
            } else if self.integrator == 0 && self.pressed {
                self.pressed = false;
                return false;
            }

            // Wait for the next sample interval
            Timer::after(self.sample_interval).await;
        }
    }
}

/// Default Debouncer with a threshold of 3, sampling interval of 10ms and active low.
impl Default for Debouncer {
    fn default() -> Self {
        Self {
            integrator: 0,
            threshold: 3,
            sample_interval: Duration::from_millis(10),
            active_state: ActiveState::ActiveLow,
            pressed: false,
        }
    }
}
