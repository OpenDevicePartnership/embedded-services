//! Debounce Module

use embedded_hal::digital::InputPin;
use embedded_hal_async::delay::DelayNs;
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
    sample_interval: u32,
    active_state: ActiveState,
    pressed: bool,
}

impl Debouncer {
    /// Creates a new Debouncer instance with the given threshold value, sampling interval and active state.
    pub fn new(threshold: u8, sample_interval: u32, active_state: ActiveState) -> Self {
        Self {
            integrator: 0,
            threshold,
            sample_interval,
            active_state,
            pressed: false,
        }
    }

    /// Debounces a button press using an integrator.
    pub async fn debounce<D: DelayNs, I: InputPin + Wait>(&mut self, gpio: &mut I, delay: &mut D) -> bool {
        let previous_pressed = self.pressed;

        loop {
            self.update(gpio);

            if self.pressed != previous_pressed {
                return self.pressed;
            }

            // Wait for the next sample interval
            delay.delay_ms(self.sample_interval).await;
        }
    }

    fn update<I: InputPin>(&mut self, gpio: &mut I) {
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
        if self.integrator == self.threshold && !self.pressed {
            self.pressed = true;
        } else if self.integrator == 0 && self.pressed {
            self.pressed = false;
        }
    }
}

/// Default Debouncer with a threshold of 3, sampling interval of 10ms and active low.
impl Default for Debouncer {
    fn default() -> Self {
        Self {
            integrator: 0,
            threshold: 3,
            sample_interval: 10,
            active_state: ActiveState::ActiveLow,
            pressed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use embedded_hal_mock::eh1::{
        delay::{CheckedDelay, Transaction as DelayTransation},
        digital::{Mock, State, Transaction as PinTransaction},
    };

    #[test]
    fn test_single_update_active_low_press() {
        let mut d = Debouncer::default();

        let gpio_expectations = [PinTransaction::get(State::Low)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, 1);
        assert!(!d.pressed);

        gpio.done();
    }

    #[test]
    fn test_single_update_active_high_press() {
        let mut d = Debouncer::default();
        d.active_state = ActiveState::ActiveHigh;

        let gpio_expectations = [PinTransaction::get(State::High)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, 1);
        assert!(!d.pressed);

        gpio.done();
    }

    #[test]
    fn test_last_update_state_change() {
        // Press, active high
        let mut d = Debouncer::default();
        d.integrator = d.threshold - 1;

        let gpio_expectations = [PinTransaction::get(State::Low)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, d.threshold);
        assert!(d.pressed);

        gpio.done();

        // Release, active high
        d.integrator = 1;

        let gpio_expectations = [PinTransaction::get(State::High)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, 0);
        assert!(!d.pressed);

        gpio.done();

        // Press, active low
        d.integrator = d.threshold - 1;
        d.active_state = ActiveState::ActiveLow;

        let gpio_expectations = [PinTransaction::get(State::Low)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, d.threshold);
        assert!(d.pressed);

        gpio.done();

        // Release, active low
        d.integrator = 1;
        d.active_state = ActiveState::ActiveLow;

        let gpio_expectations = [PinTransaction::get(State::High)];
        let mut gpio = Mock::new(&gpio_expectations);

        d.update(&mut gpio);

        assert_eq!(d.integrator, 0);
        assert!(!d.pressed);

        gpio.done();
    }

    #[tokio::test]
    async fn test_stable_first_press() {
        let mut d = Debouncer::default();

        let gpio_expectations = [
            PinTransaction::get(State::Low),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::Low),
        ];
        let mut gpio = Mock::new(&gpio_expectations);

        let delay_expectations = [
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            // Last iter: we return before delaying again
        ];
        let mut delay = CheckedDelay::new(&delay_expectations);

        let pressed = d.debounce(&mut gpio, &mut delay).await;

        assert!(pressed);
        assert_eq!(d.integrator, d.threshold);
        assert!(d.pressed);

        gpio.done();
        delay.done();
    }

    #[tokio::test]
    async fn test_stable_first_release() {
        let mut d = Debouncer::default();
        d.pressed = true;
        d.integrator = d.threshold;

        let gpio_expectations = [
            PinTransaction::get(State::High),
            PinTransaction::get(State::High),
            PinTransaction::get(State::High),
        ];
        let mut gpio = Mock::new(&gpio_expectations);

        let delay_expectations = [
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            // Last iter: we return before delaying again
        ];
        let mut delay = CheckedDelay::new(&delay_expectations);

        let pressed = d.debounce(&mut gpio, &mut delay).await;

        assert!(!pressed);
        assert!(!d.pressed);
        assert_eq!(d.integrator, 0);

        gpio.done();
        delay.done();
    }

    #[tokio::test]
    async fn test_bounces_settle_press() {
        let mut d = Debouncer::default();

        let gpio_expectations = [
            PinTransaction::get(State::Low),
            PinTransaction::get(State::High),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::High),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::Low),
        ];
        let mut gpio = Mock::new(&gpio_expectations);

        let delay_expectations = [
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            // Last iter: we return before delaying again
        ];
        let mut delay = CheckedDelay::new(&delay_expectations);

        let pressed = d.debounce(&mut gpio, &mut delay).await;

        assert!(pressed);
        assert!(d.pressed);
        assert_eq!(d.integrator, d.threshold);

        gpio.done();
        delay.done();
    }

    #[tokio::test]
    async fn test_bounces_settle_released() {
        let mut d = Debouncer::default();
        d.integrator = d.threshold;
        d.pressed = true;

        let gpio_expectations = [
            PinTransaction::get(State::Low),
            PinTransaction::get(State::High),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::Low),
            PinTransaction::get(State::High),
            PinTransaction::get(State::High),
            PinTransaction::get(State::High),
        ];
        let mut gpio = Mock::new(&gpio_expectations);

        let delay_expectations = [
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            // Last iter: we return before delaying again
        ];
        let mut delay = CheckedDelay::new(&delay_expectations);

        let pressed = d.debounce(&mut gpio, &mut delay).await;

        assert!(!pressed);
        assert!(!d.pressed);
        assert_eq!(d.integrator, 0);

        gpio.done();
        delay.done();
    }

    #[tokio::test]
    async fn test_bounces_high_threshold() {
        let mut d = Debouncer::default();
        d.threshold = 10;

        let gpio_expectations = [
            PinTransaction::get(State::Low),  // 1
            PinTransaction::get(State::High), // 0
            PinTransaction::get(State::Low),  // 1
            PinTransaction::get(State::Low),  // 2
            PinTransaction::get(State::High), // 1
            PinTransaction::get(State::High), // 0
            PinTransaction::get(State::High), // 0
            PinTransaction::get(State::Low),  // 1
            PinTransaction::get(State::Low),  // 2
            PinTransaction::get(State::Low),  // 3
            PinTransaction::get(State::Low),  // 4
            PinTransaction::get(State::Low),  // 5
            PinTransaction::get(State::Low),  // 6
            PinTransaction::get(State::Low),  // 7
            PinTransaction::get(State::Low),  // 8
            PinTransaction::get(State::Low),  // 9
            PinTransaction::get(State::Low),  // 10
        ];
        let mut gpio = Mock::new(&gpio_expectations);

        let delay_expectations = [
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            DelayTransation::delay_ms(10),
            // final iter: we return before delaying again
        ];
        let mut delay = CheckedDelay::new(&delay_expectations);

        let pressed = d.debounce(&mut gpio, &mut delay).await;

        assert!(pressed);
        assert!(d.pressed);
        assert_eq!(d.integrator, d.threshold);

        gpio.done();
        delay.done();
    }
}
