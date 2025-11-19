//! Embedded Services Interface Exports

#![no_std]
#![warn(missing_docs)]

pub mod intrusive_list;
pub use intrusive_list::*;

pub mod critical_section_cell;
#[cfg(all(not(test), target_os = "none", target_arch = "arm"))]
pub mod thread_mode_cell;

/// short-hand include all pre-baked services
pub mod activity;
pub mod broadcaster;
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
pub mod sync;
pub mod type_c;

/// Global Mutex type, ThreadModeRawMutex is used in a microcontroller context, whereas CriticalSectionRawMutex is used
/// in a standard context for unit testing.
///
/// Used because ThreadModeRawMutex is not unit test friendly
/// but CriticalSectionRawMutex would incur a significant performance impact, since it disables interrupts.
#[cfg(any(test, not(target_os = "none"), target_arch = "riscv32"))]
pub type GlobalRawMutex = embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
/// Global Mutex type, ThreadModeRawMutex is used in a microcontroller context, whereas CriticalSectionRawMutex is used
/// in a standard context for unit testing.
///
/// Used because ThreadModeRawMutex is not unit test friendly
/// but CriticalSectionRawMutex would incur a significant performance impact, since it disables interrupts.
#[cfg(all(not(test), target_os = "none", not(target_arch = "riscv32")))]
pub type GlobalRawMutex = embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;

/// AtomicUsize and Ordering re-exports. Uses core::sync::atomic if the target supports atomic operations,
/// otherwise falls back to portable-atomic crate.
#[cfg(target_has_atomic = "ptr")]
pub use core::sync::atomic::AtomicUsize;
#[cfg(target_has_atomic = "ptr")]
pub use core::sync::atomic::Ordering;
#[cfg(not(target_has_atomic = "ptr"))]
pub use portable_atomic::AtomicUsize;
#[cfg(not(target_has_atomic = "ptr"))]
pub use portable_atomic::Ordering;

/// A cell type that is Sync and Send. CriticalSectionCell is used in a standard context to support multiple cores and
/// executors.
#[cfg(any(test, not(target_os = "none"), all(target_os = "none", not(target_arch = "arm"))))]
pub type SyncCell<T> = critical_section_cell::CriticalSectionCell<T>;

/// ThreadModeCell is leaner and used in a microcontroller context for when there's a guarantee of a
/// single core and executor. Only supports ARM Cortex-M architecture.
#[cfg(all(not(test), target_os = "none", target_arch = "arm"))]
pub type SyncCell<T> = thread_mode_cell::ThreadModeCell<T>;

/// initialize all service static interfaces as required. Ideally, this is done before subsystem initialization
#[allow(clippy::unused_async)]
pub async fn init() {
    comms::init();
    activity::init();
    hid::init();
    cfu::init();
    keyboard::init();
    power::policy::init();
    type_c::controller::init();
}
