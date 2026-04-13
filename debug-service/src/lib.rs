#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

#[cfg(feature = "defmt")]
mod debug_service;
#[cfg(feature = "defmt")]
mod defmt_ring_logger;
#[cfg(feature = "defmt")]
pub mod task;

#[cfg(feature = "defmt")]
pub use debug_service::*;
