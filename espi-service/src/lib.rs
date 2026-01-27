#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

// This module has a hard dependency on embassy-imxrt, which doesn't work on desktop.
// This means that the entire workspace's tests won't compile if this module is enabled.
//
// On Linux, we sort-of get away with it - as far as I can tell, the linker on Linux is more aggressive
// with pruning unused code, so as long as there's no test that calls into anything that eventually calls
// into embassy-imxrt, we at least compile on Linux.
//
// However, on Windows, it looks like the linker is erroring out because it can't find embassy-imxrt-related
// symbols before it does the analysis to determine that those symbols aren't reachable anyway, so we have to
// disable this module entirely to be able to compile the workspace's tests at all on Windows.
//
// If we ever want to run tests for this module on Windows, we'll need some way to break the dependency
// on embassy-imxrt - probably by switching to some sort of trait-based interface with eSPI.  Until then,
// we need to gate everything on #[cfg(not(test))].

#[cfg(not(test))]
mod espi_service;

#[cfg(not(test))]
mod mctp;

#[cfg(not(test))]
pub mod task;

#[cfg(not(test))]
pub use espi_service::*;
