//! Embedded Services Interface Exports

#![no_std]
#![warn(missing_docs)]

pub mod intrusive_list;
pub use intrusive_list::*;

pub mod critical_section_cell;

/// short-hand include all pre-baked services
pub mod activity;
pub mod buffer;
pub mod cfu;
pub mod comms;
pub mod ec_type;
pub mod fmt;
pub mod hid;
pub mod init;
pub mod ipc;
pub mod keyboard;
pub mod power;
pub mod type_c;

/// Global Mutex type, ThreadModeRawMutex is used in a microcontroller context, whereas CriticalSectionRawMutex is used
/// in a standard context for unit testing.
///
/// Used because ThreadModeRawMutex is not unit test friendly
/// but CriticalSectionRawMutex would incur a significant performance impact, since it disables interrupts.
#[cfg(any(test, not(target_os = "none")))]
pub type GlobalRawMutex = embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
/// Global Mutex type, ThreadModeRawMutex is used in a microcontroller context, whereas CriticalSectionRawMutex is used
/// in a standard context for unit testing.
///
/// Used because ThreadModeRawMutex is not unit test friendly
/// but CriticalSectionRawMutex would incur a significant performance impact, since it disables interrupts.
#[cfg(all(not(test), target_os = "none", not(feature = "mpssfh")))]
pub type GlobalRawMutex = embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;

#[cfg(all(not(test), target_os = "none", feature = "mpssfh"))]
pub type GlobalRawMutex = embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

/// A cell type that is Sync and Send. CriticalSectionCell is used in a standard context to support multiple cores and
/// executors, whereas ThreadModeCell is leaner and used in a microcontroller context for when there's a guarantee of a
/// single core and executor.
// #[cfg(any(test, not(target_os = "none")))]
pub type SyncCell<T> = critical_section_cell::CriticalSectionCell<T>;

/// initialize all service static interfaces as required. Ideally, this is done before subsystem initialization
pub async fn init() {
    comms::init();
    activity::init();
    hid::init();
    cfu::init();
    keyboard::init();
    power::policy::init();
    type_c::controller::init();
}
