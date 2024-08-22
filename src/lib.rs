//! Embedded Services Interface Exports

#![no_std]
#![warn(missing_docs)]

// pub struct Subscription<T> {

// }

/// export activity services
pub mod activity;

/// initialize all services
pub fn init() {
    activity::init();
}
