#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

#[cfg(feature = "imxrt")]
mod espi_service;
#[cfg(feature = "imxrt")]
pub mod task;

#[cfg(feature = "imxrt")]
pub use espi_service::*;
