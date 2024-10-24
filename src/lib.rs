//! Embedded Services Interface Exports

#![no_std]
#![warn(missing_docs)]

pub mod intrusive_list;
pub use intrusive_list::*;

/// short-hand include all pre-baked services
pub mod activity;
pub mod fmt;
pub mod hid;
pub mod transport;

/// initialize all service static interfaces as required. Ideally, this is done before subsystem initialization
pub async fn init() {
    transport::init();
    activity::init();
    hid::init();
}
