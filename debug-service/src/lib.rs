#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

mod debug_service;
mod defmt_ring_logger;
pub mod task;

pub use debug_service::*;
