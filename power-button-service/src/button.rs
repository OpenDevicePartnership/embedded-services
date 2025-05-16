//! Button Service Definitions

use embassy_time::{with_timeout, Delay, Duration, Instant, TimeoutError, Timer};
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;

use super::debounce::Debouncer;

#[derive(Debug)]
/// A struct representing a button with a generic GPIO pin and a debouncer.
pub struct Button<I> {
    gpio: I,
    config: ButtonConfig,
}

#[derive(Debug)]
/// Struct representing the configuration for a button.
pub struct ButtonConfig {
    debouncer: Debouncer,
    short_press_threshold: Duration,
    timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
/// Enum representing the state of a button.
pub enum ButtonState {
    /// The button is released and `Instant` represents the time when it was released.
    ButtonReleased(Instant),
    /// The button is pressed and `Instant` represents the time when it was pressed.
    ButtonPressed(Instant),
}

impl<I: InputPin + Wait> Button<I> {
    /// Creates a new `Button` instance with the given GPIO pin.
    pub fn new(gpio: I, config: ButtonConfig) -> Self {
        Self { gpio, config }
    }

    /// Returns the button configuration.
    pub fn get_config(&self) -> &ButtonConfig {
        &self.config
    }

    /// Sets the button configuration.
    pub fn set_config(&mut self, config: ButtonConfig) {
        self.config = config;
    }

    /// Checks button state.
    pub async fn get_button_state(&mut self) -> ButtonState {
        let mut delay = Delay;

        match self.config.debouncer.debounce(&mut self.gpio, &mut delay).await {
            true => ButtonState::ButtonPressed(Instant::now()),
            false => ButtonState::ButtonReleased(Instant::now()),
        }
    }

    /// Asynchronously gets the duration for which the button was pressed.
    pub async fn get_press_duration(&mut self) -> Option<Duration> {
        let timeout = self.config.timeout;

        // Wait for the button to be pressed
        if let ButtonState::ButtonPressed(_) = self.get_button_state().await {
            // Record the timestamp when the button is pressed
            let start = Instant::now();

            let release_future = async {
                while let ButtonState::ButtonPressed(_) = self.get_button_state().await {
                    Timer::after(Duration::from_millis(10)).await;
                }
                Instant::now()
            };

            // Wait for the button to be released or timeout
            let end = with_timeout(timeout, release_future).await;

            return Some(match end {
                Ok(end) => end - start,
                Err(TimeoutError) => Instant::now() - start,
            });
        }

        None
    }
}

impl ButtonConfig {
    /// Creates a new ButtonConfig instance with the default values.
    pub fn new(debouncer: Debouncer, short_press_threshold: Duration, timeout: Duration) -> Self {
        Self {
            debouncer,
            short_press_threshold,
            timeout,
        }
    }

    /// Gets the timeout duration.
    pub fn get_timeout(&self) -> Duration {
        self.timeout
    }

    /// Gets the short press threshold duration.
    pub fn get_short_press_threshold(&self) -> Duration {
        self.short_press_threshold
    }
}

impl Default for ButtonConfig {
    fn default() -> Self {
        Self {
            debouncer: Debouncer::default(),
            short_press_threshold: Duration::from_millis(2000),
            timeout: Duration::from_millis(5000),
        }
    }
}
