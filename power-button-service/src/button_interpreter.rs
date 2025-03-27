//! Button Interpreter Module

use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;

use super::button::Button;

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enum representing the different types of messages that can be sent by the button.
pub enum Message {
    /// Button pressed for long duration.
    LongPress,
    /// Button pressed for short duration.
    ShortPress,
    /// Button pressed and held.
    PressAndHold,
}

/// Checks the button press duration and returns the corresponding state.
pub async fn check_button_press<I: InputPin + Wait>(button: &mut Button<I>) -> Option<Message> {
    let timeout = button.get_config().get_timeout();
    let short_press_threshold = button.get_config().get_short_press_threshold();

    if let Some(duration) = button.get_press_duration().await {
        if duration.as_millis() >= timeout.as_millis() {
            Some(Message::PressAndHold)
        } else if duration.as_millis() >= short_press_threshold.as_millis() {
            Some(Message::LongPress)
        } else {
            Some(Message::ShortPress)
        }
    } else {
        // Ignore button release which timed out
        None
    }
}
