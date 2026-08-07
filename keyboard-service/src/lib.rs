//! Keyboard Service
//!
//! Provides a simple keyboard service interface, a GPIO key-matrix implementation, and
//! a reusable HID relay adapter.
//!
//! For more complicated keyboards (e.g. with media keys, buttons that don't get exposed to host, NKRO, etc),
//! you may need something custom.
#![no_std]

pub mod gpio_kb;
pub mod interface;
pub mod relay;

pub use gpio_kb::{
    KeyboardConfig, KeyboardError, KeyboardInitError, Layers, LedConfig, Resources, Runner, Service, layout,
};
pub use interface::{KeyboardInputReport, KeyboardPowerState, KeyboardService, LedFlags};
pub use relay::KeyboardHidRelay;
